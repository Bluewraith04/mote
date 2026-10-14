//! Registry packages through `mote sync` and `mote add`, against a fake registry.

mod common;

use std::fs;
use std::path::Path;

use common::Fake;
use pkg::{Lockfile, MpkArchive, PackageManager};

fn manifest_text(app: &Path) -> String {
    fs::read_to_string(app.join("mote.toml")).unwrap()
}

fn lock(app: &Path) -> Lockfile {
    Lockfile::from_file(&app.join("mote.lock")).unwrap()
}

#[test]
fn add_writes_a_caret_on_the_highest_release_and_installs_it() {
    let fake = Fake::start();
    fake.publish("shapes", "1.0.0", &[], "pub fn old() {}");
    let sum = fake.publish("shapes", "1.2.3", &[], "pub fn area() {}");
    fake.publish("shapes", "1.3.0", &[], "pub fn yanked() {}");
    fake.yank("shapes", "1.3.0");
    fake.publish("shapes", "2.0.0-rc.1", &[], "pub fn pre() {}");
    let app = fake.app("add", "");
    PackageManager::add_dependency(&app, "shapes").unwrap();

    assert!(manifest_text(&app).contains("shapes = \"^1.2.3\""), "{}", manifest_text(&app));
    let installed = app.join(".mote_packages/shapes");
    assert_eq!(fs::read_to_string(installed.join("src/lib.mote")).unwrap(), "pub fn area() {}");
    assert!(!installed.join("shapes.mbc").exists() && !installed.join("interface.txt").exists());
    let lock = lock(&app);
    let p = &lock.packages[0];
    assert_eq!((p.name.as_str(), p.version.to_string()), ("shapes", "1.2.3".to_string()));
    assert_eq!(p.source, format!("registry+{}#{sum}", fake.url));
    assert!(p.checksum.as_deref().is_some_and(|c| c.starts_with("sha256:")));
}

#[test]
fn add_with_a_version_writes_that_requirement() {
    let fake = Fake::start();
    fake.publish("shapes", "1.2.0", &[], "pub fn a() {}");
    fake.publish("shapes", "1.2.5", &[], "pub fn b() {}");
    fake.publish("shapes", "1.3.0", &[], "pub fn c() {}");
    let app = fake.app("addat", "");
    PackageManager::add_dependency(&app, "shapes@1.2").unwrap();
    assert!(manifest_text(&app).contains("shapes = \"^1.2\""), "{}", manifest_text(&app));
    assert_eq!(lock(&app).packages[0].version.to_string(), "1.3.0");

    let err = PackageManager::add_dependency(&app, "shapes@^7.0.0").unwrap_err();
    assert!(err.contains("no version of shapes"), "{err}");
}

#[test]
fn a_failed_add_leaves_the_manifest_as_it_was() {
    let fake = Fake::start();
    let before = fake.app("rollback", "");
    let text = manifest_text(&before);
    assert!(PackageManager::add_dependency(&before, "missing").unwrap_err().contains("no package `missing`"));
    assert_eq!(manifest_text(&before), text);
}

#[test]
fn sync_resolves_requirements_and_transitive_dependencies() {
    let fake = Fake::start();
    fake.publish("base", "1.0.0", &[], "pub fn one() {}");
    fake.publish("base", "1.4.0", &[], "pub fn four() {}");
    fake.publish("base", "2.0.0", &[], "pub fn two() {}");
    fake.publish("top", "0.3.0", &[("base", "^1.0.0")], "pub fn top() {}");
    let app = fake.app("transitive", "top = \"^0.3\"\n");
    let resolved = PackageManager::install_dependencies(&app, false).unwrap();
    let names: Vec<_> = resolved.iter().map(|r| format!("{} {}", r.name, r.version)).collect();
    assert_eq!(names, ["base 1.4.0", "top 0.3.0"]);
    assert!(app.join(".mote_packages/base/src/lib.mote").is_file() && app.join(".mote_packages/top/src/lib.mote").is_file());
}

#[test]
fn a_lock_keeps_its_version_and_needs_no_registry_while_the_files_are_installed() {
    let fake = Fake::start();
    fake.publish("shapes", "1.0.0", &[], "pub fn a() {}");
    let app = fake.app("pinned", "shapes = \"^1.0\"\n");
    PackageManager::install_dependencies(&app, false).unwrap();
    let before = fake.hits().len();

    fake.publish("shapes", "1.1.0", &[], "pub fn b() {}");
    PackageManager::install_dependencies(&app, false).unwrap();
    assert_eq!(lock(&app).packages[0].version.to_string(), "1.0.0", "sync does not upgrade");
    assert_eq!(fake.hits().len(), before, "nothing was fetched");

    fake.stop();
    PackageManager::install_dependencies(&app, true).unwrap();
}

#[test]
fn a_package_fetched_once_installs_again_offline() {
    let fake = Fake::start();
    fake.publish("shapes", "1.0.0", &[], "pub fn a() {}");
    let app = fake.app("offline", "shapes = \"^1.0\"\n");
    PackageManager::install_dependencies(&app, false).unwrap();
    fake.stop();

    fs::remove_dir_all(app.join(".mote_packages")).unwrap();
    PackageManager::install_dependencies(&app, true).unwrap();
    assert!(app.join(".mote_packages/shapes/src/lib.mote").is_file());

    fs::remove_file(app.join("mote.lock")).unwrap();
    PackageManager::install_dependencies(&app, false).unwrap();
    assert_eq!(lock(&app).packages[0].version.to_string(), "1.0.0", "the kept index answers");
}

#[test]
fn a_yanked_version_is_not_chosen_but_a_lock_may_keep_it() {
    let fake = Fake::start();
    fake.publish("shapes", "1.0.0", &[], "pub fn a() {}");
    fake.publish("shapes", "1.1.0", &[], "pub fn b() {}");
    let app = fake.app("yank", "shapes = \"^1.0\"\n");
    PackageManager::install_dependencies(&app, false).unwrap();
    assert_eq!(lock(&app).packages[0].version.to_string(), "1.1.0");

    fake.yank("shapes", "1.1.0");
    fs::remove_dir_all(app.join(".mote_packages")).unwrap();
    PackageManager::install_dependencies(&app, false).unwrap();
    assert_eq!(lock(&app).packages[0].version.to_string(), "1.1.0", "the lock still holds the yanked version");

    fs::remove_file(app.join("mote.lock")).unwrap();
    PackageManager::install_dependencies(&app, false).unwrap();
    assert_eq!(lock(&app).packages[0].version.to_string(), "1.0.0");
}

#[test]
fn a_pre_release_is_chosen_only_when_the_requirement_names_one() {
    let fake = Fake::start();
    fake.publish("shapes", "1.0.0", &[], "pub fn a() {}");
    fake.publish("shapes", "1.1.0-rc.1", &[], "pub fn b() {}");
    let app = fake.app("pre", "shapes = \"^1.0\"\n");
    PackageManager::install_dependencies(&app, false).unwrap();
    assert_eq!(lock(&app).packages[0].version.to_string(), "1.0.0");

    fs::remove_file(app.join("mote.lock")).unwrap();
    fs::write(app.join("mote.toml"), manifest_text(&app).replace("\"^1.0\"", "\"^1.1.0-rc.1\"")).unwrap();
    PackageManager::install_dependencies(&app, false).unwrap();
    assert_eq!(lock(&app).packages[0].version.to_string(), "1.1.0-rc.1");
}

#[test]
fn an_archive_that_is_not_what_the_index_promised_is_refused() {
    let fake = Fake::start();
    fake.publish("shapes", "1.0.0", &[], "pub fn a() {}");
    fake.state.lock().unwrap().tamper = true;
    let app = fake.app("tamper", "shapes = \"^1.0\"\n");
    let err = PackageManager::install_dependencies(&app, false).unwrap_err();
    assert!(err.contains("the registry's index says"), "{err}");
    assert!(!app.join(".mote_packages/shapes").exists());
}

#[test]
fn an_archive_for_another_package_is_refused() {
    let fake = Fake::start();
    fake.publish("other", "1.0.0", &[], "pub fn a() {}");
    let other = fake.state.lock().unwrap().packages["other"][0].bytes.clone();
    fake.publish_archive("shapes", "1.0.0", &[], other);
    let app = fake.app("wrongname", "shapes = \"^1.0\"\n");
    let err = PackageManager::install_dependencies(&app, false).unwrap_err();
    assert!(err.contains("the archive holds other 1.0.0"), "{err}");
}

#[test]
fn an_archive_path_that_climbs_out_is_refused() {
    let fake = Fake::start();
    fake.publish_files("shapes", "1.0.0", &[], &[("src/lib.mote", ""), ("src/../../escape.txt", "x")]);
    let app = fake.app("climb", "shapes = \"^1.0\"\n");
    let err = PackageManager::install_dependencies(&app, false).unwrap_err();
    assert!(err.contains("the archive has the path"), "{err}");
    assert!(!app.join("escape.txt").exists() && !app.join(".mote_packages/escape.txt").exists());
}

#[test]
fn a_token_in_mote_toml_is_an_error() {
    let err = pkg::PackageManifest::from_toml_str("[package]\nname = \"a\"\nversion = \"0.1.0\"\n\n[registries]\nmain = { url = \"https://r.example\", token = \"secret\" }\n").unwrap_err();
    assert!(err.contains("~/.mote/mote_config.toml"), "{err}");
}

#[test]
fn a_registry_package_and_a_path_package_cannot_share_a_name() {
    let fake = Fake::start();
    fake.publish("gadget", "1.0.0", &[], "pub fn g() {}");
    fake.publish("top", "1.0.0", &[("gadget", "^1.0")], "pub fn t() {}");
    let app = fake.app("both", "top = \"^1.0\"\n");
    let other = std::env::temp_dir().join(format!("mote_registry_both_dep_{}", std::process::id()));
    fs::create_dir_all(other.join("src")).unwrap();
    fs::write(other.join("mote.toml"), "[package]\nname = \"gadget\"\nversion = \"1.0.0\"\n\n[dependencies]\n").unwrap();
    fs::write(app.join("mote.toml"), manifest_text(&app).replace("top = \"^1.0\"\n", &format!("top = \"^1.0\"\ngadget = {{ path = \"{}\" }}\n", other.display()))).unwrap();
    let err = PackageManager::install_dependencies(&app, false).unwrap_err();
    assert!(err.contains("gadget comes from a registry and from a git repository or path"), "{err}");
}

#[test]
fn a_registry_package_may_not_depend_on_a_path() {
    let fake = Fake::start();
    let manifest = "[package]\nname = \"top\"\nversion = \"1.0.0\"\n\n[dependencies]\nlocal = { path = \"../local\" }\n";
    let bytes = MpkArchive::encode(&[("mote.toml".to_string(), manifest.as_bytes().to_vec()), ("src/lib.mote".to_string(), vec![])]).unwrap();
    fake.publish_archive("top", "1.0.0", &[], bytes);
    let app = fake.app("pathdep", "top = \"^1.0\"\n");
    let err = PackageManager::install_dependencies(&app, false).unwrap_err();
    assert!(err.contains("a registry package can only depend on registry packages"), "{err}");
}

