//! Packages that carry `native/<triple>/` libraries, through `mote install`, against repositories and directories on disk.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use pkg::native::host_triple;
use pkg::{Lockfile, PackageManager};

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("mote_native_{name}_{}", std::process::id()));
    fs::remove_dir_all(&root).ok();
    fs::create_dir_all(&root).unwrap();
    root
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A repository `lib` at tag `v1.0.0` holding a `lib.mote` and one `native/<triple>/libx` per triple in `triples`.
fn publish(root: &Path, triples: &[&str]) -> String {
    let dir = root.join("lib");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("mote.toml"), "[package]\nname = \"lib\"\nversion = \"1.0.0\"\n").unwrap();
    fs::write(dir.join("src/lib.mote"), "pub fn v() -> Int { return 1 }\n").unwrap();
    for triple in triples {
        fs::create_dir_all(dir.join("native").join(triple)).unwrap();
        fs::write(dir.join("native").join(triple).join("libx.bin"), format!("code for {triple}")).unwrap();
    }
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "c"]);
    git(&dir, &["tag", "-a", "-m", "v1.0.0", "v1.0.0"]);
    format!("file://{}", dir.display())
}

fn make_app(root: &Path, dep: &str) -> PathBuf {
    let app = root.join("app");
    fs::remove_dir_all(&app).ok();
    fs::create_dir_all(app.join("src")).unwrap();
    fs::write(app.join("mote.toml"), format!("[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n{dep}\n")).unwrap();
    app
}

#[test]
fn a_granted_package_installs_only_this_machines_libraries_and_the_lock_checksums_each_triple() {
    let root = scratch("granted");
    let host = host_triple();
    let url = publish(&root, &[&host, "other-unknown-triple"]);
    let app = make_app(&root, &format!("lib = {{ git = \"{url}\", tag = \"v1.0.0\", native = true }}"));

    PackageManager::install_dependencies(&app, false).unwrap();
    let lock = Lockfile::from_file(&app.join("mote.lock")).unwrap();
    let triples: Vec<&String> = lock.packages[0].native.keys().collect();
    assert_eq!(triples.len(), 2);
    assert!(lock.packages[0].native.contains_key(&host));
    assert!(lock.packages[0].native.values().all(|c| c.starts_with("sha256:")));
    assert!(app.join(".mote_packages/lib/native").join(&host).join("libx.bin").is_file());
    assert!(!app.join(".mote_packages/lib/native/other-unknown-triple").exists(), "only the host's directory is unpacked");

    PackageManager::install_dependencies(&app, true).unwrap();

    fs::write(app.join(".mote_packages/lib/native").join(&host).join("libx.bin"), b"edited").unwrap();
    PackageManager::install_dependencies(&app, true).unwrap();
    let restored = fs::read_to_string(app.join(".mote_packages/lib/native").join(&host).join("libx.bin")).unwrap();
    assert_eq!(restored, format!("code for {host}"), "a hand-edited library is fetched again");
    fs::remove_dir_all(&root).ok();
}

#[test]
fn native_code_without_a_grant_is_refused() {
    let root = scratch("ungranted");
    let host = host_triple();
    let url = publish(&root, &[&host]);
    let app = make_app(&root, &format!("lib = {{ git = \"{url}\", tag = \"v1.0.0\" }}"));
    let err = PackageManager::install_dependencies(&app, false).unwrap_err();
    assert!(err.contains("lib carries native code; list it in mote.toml with native = true"), "{err}");
    assert!(!app.join(".mote_packages/lib").exists());
    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_package_with_no_library_for_this_machine_is_refused() {
    let root = scratch("wrong_triple");
    let url = publish(&root, &["other-unknown-triple"]);
    let app = make_app(&root, &format!("lib = {{ git = \"{url}\", tag = \"v1.0.0\", native = true }}"));
    let err = PackageManager::install_dependencies(&app, false).unwrap_err();
    assert!(err.contains(&format!("lib has no native code for {}", host_triple())), "{err}");
    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_changed_library_fails_against_the_lock() {
    let root = scratch("changed");
    let host = host_triple();
    let url = publish(&root, &[&host]);
    let app = make_app(&root, &format!("lib = {{ git = \"{url}\", tag = \"v1.0.0\", native = true }}"));
    PackageManager::install_dependencies(&app, false).unwrap();
    let lock = fs::read_to_string(app.join("mote.lock")).unwrap();
    let at = lock.rfind("sha256:").unwrap() + "sha256:".len();
    let flipped = if lock[at..].starts_with('0') { '1' } else { '0' };
    fs::write(app.join("mote.lock"), format!("{}{flipped}{}", &lock[..at], &lock[at + 1..])).unwrap();
    fs::remove_dir_all(app.join(".mote_packages")).unwrap();
    let err = PackageManager::install_dependencies(&app, false).unwrap_err();
    assert!(err.contains("the native files at"), "{err}");
    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_path_package_copies_this_machines_libraries_and_needs_the_grant() {
    let root = scratch("path");
    let host = host_triple();
    let lib = root.join("lib");
    fs::create_dir_all(lib.join("src")).unwrap();
    fs::create_dir_all(lib.join("native").join(&host)).unwrap();
    fs::create_dir_all(lib.join("native/other-unknown-triple")).unwrap();
    fs::write(lib.join("mote.toml"), "[package]\nname = \"lib\"\nversion = \"1.0.0\"\n").unwrap();
    fs::write(lib.join("src/lib.mote"), "pub fn v() -> Int { return 1 }\n").unwrap();
    fs::write(lib.join("native").join(&host).join("libx.bin"), b"here").unwrap();
    fs::write(lib.join("native/other-unknown-triple/libx.bin"), b"elsewhere").unwrap();

    let app = make_app(&root, "lib = { path = \"../lib\" }");
    let err = PackageManager::install_dependencies(&app, false).unwrap_err();
    assert!(err.contains("lib carries native code"), "{err}");

    let app = make_app(&root, "lib = { path = \"../lib\", native = true }");
    PackageManager::install_dependencies(&app, false).unwrap();
    assert!(app.join(".mote_packages/lib/native").join(&host).join("libx.bin").is_file());
    assert!(!app.join(".mote_packages/lib/native/other-unknown-triple").exists());
    let lock = Lockfile::from_file(&app.join("mote.lock")).unwrap();
    assert_eq!(lock.packages[0].native.len(), 2);
    assert_eq!(pkg::native::grants(&app), vec![("lib".to_string(), app.join(".mote_packages/lib/native").join(&host))]);
    fs::remove_dir_all(&root).ok();
}
