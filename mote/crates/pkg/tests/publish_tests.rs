//! `mote publish` against a fake registry on localhost.

mod common;

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use common::{Fake, TOKEN};
use pkg::publish::publish;
use pkg::{MpkArchive, PackageManager};

/// Takes the turn and sets the token (`None` removes it).
fn turn(token: Option<&str>) -> MutexGuard<'static, ()> {
    static TURN: Mutex<()> = Mutex::new(());
    let guard = TURN.lock().unwrap_or_else(|e| e.into_inner());
    common::isolate_home();
    match token {
        Some(t) => std::env::set_var("MOTE_REGISTRY_TOKEN", t),
        None => std::env::remove_var("MOTE_REGISTRY_TOKEN"),
    }
    guard
}

/// A package `name` at 1.0.0 with a `README.md` and one source file.
fn project(fake: &Fake, name: &str, meta: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mote_publish_{name}_{}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(dir.join("src")).unwrap();
    let manifest = format!("[package]\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"src/lib.mote\"\n{meta}\n[registries]\nmain = \"{}\"\n", fake.url);
    fs::write(dir.join("mote.toml"), manifest).unwrap();
    fs::write(dir.join("src/lib.mote"), "pub fn hello() -> String {\n    return \"hi\"\n}\n").unwrap();
    fs::write(dir.join("README.md"), "# readme\n").unwrap();
    dir
}

const META: &str = "description = \"Greets\"\nlicense = \"MIT\"\n";

fn names(bytes: &[u8]) -> Vec<String> {
    MpkArchive::decode(bytes).unwrap().into_iter().map(|(p, _)| p).collect()
}

#[test]
fn a_package_is_published_and_then_added_from_the_registry() {
    let _turn = turn(Some(TOKEN));
    let fake = Fake::start();
    let dir = project(&fake, "greeter", META);
    let done = publish(&dir, None).unwrap();
    assert_eq!((done.name.as_str(), done.version.to_string(), done.registry.as_str()), ("greeter", "1.0.0".to_string(), "main"));

    let stored = names(&fake.archive("greeter", "1.0.0").unwrap());
    assert!(stored.contains(&"mote.toml".to_string()) && stored.contains(&"README.md".to_string()) && stored.contains(&"src/lib.mote".to_string()), "{stored:?}");

    let user = fake.app("greeter_user", "");
    PackageManager::add_dependency(&user, "greeter").unwrap();
    assert!(user.join(".mote_packages/greeter/src/lib.mote").is_file());
    assert!(user.join(".mote_packages/greeter/README.md").is_file());
}

#[test]
fn a_package_needs_a_description_and_a_license() {
    let _turn = turn(Some(TOKEN));
    let fake = Fake::start();
    let err = publish(&project(&fake, "nodesc", "license = \"MIT\"\n"), None).unwrap_err();
    assert!(err.contains("`description`"), "{err}");
    let err = publish(&project(&fake, "nolicense", "description = \"Greets\"\n"), None).unwrap_err();
    assert!(err.contains("`license`"), "{err}");
    assert!(fake.hits().iter().all(|h| !h.starts_with("PUT")));
}

#[test]
fn publishing_without_a_token_says_where_to_put_one() {
    let _turn = turn(None);
    let fake = Fake::start();
    let err = publish(&project(&fake, "notoken", META), None).unwrap_err();
    assert!(err.contains("MOTE_REGISTRY_TOKEN") && err.contains("mote_config.toml"), "{err}");
}

#[test]
fn a_token_the_registry_refuses_is_reported() {
    let _turn = turn(Some("wrong"));
    let fake = Fake::start();
    let err = publish(&project(&fake, "badtoken", META), None).unwrap_err();
    assert!(err.contains("401") && err.contains("the token is not accepted"), "{err}");
}

#[test]
fn a_version_is_never_published_twice() {
    let _turn = turn(Some(TOKEN));
    let fake = Fake::start();
    let dir = project(&fake, "twice", META);
    publish(&dir, None).unwrap();
    let err = publish(&dir, None).unwrap_err();
    assert!(err.contains("already published"), "{err}");
    assert_eq!(fake.hits().iter().filter(|h| h.starts_with("PUT")).count(), 1, "the second try never reached the registry");
}

#[test]
fn a_registry_package_cannot_depend_on_a_path_or_a_git_repository() {
    let _turn = turn(Some(TOKEN));
    let fake = Fake::start();
    let dir = project(&fake, "pathdep", META);
    let manifest = fs::read_to_string(dir.join("mote.toml")).unwrap();
    fs::write(dir.join("mote.toml"), manifest.replace("\n[registries]", "\n[dependencies]\nother = { path = \"../other\" }\n\n[registries]")).unwrap();
    let err = publish(&dir, None).unwrap_err();
    assert!(err.contains("can only depend on registry packages"), "{err}");
}

#[test]
fn reserved_and_badly_formed_names_are_refused_before_any_upload() {
    let _turn = turn(Some(TOKEN));
    let fake = Fake::start();
    for name in ["pane", "std", "x"] {
        let err = publish(&project(&fake, name, META), None).unwrap_err();
        assert!(err.contains("reserved") || err.contains("2 to 40"), "{name}: {err}");
    }
    assert!(fake.hits().is_empty());
}

#[test]
fn native_files_for_every_triple_travel_in_the_archive() {
    let _turn = turn(Some(TOKEN));
    let fake = Fake::start();
    let dir = project(&fake, "withnative", META);
    for triple in ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"] {
        fs::create_dir_all(dir.join("native").join(triple)).unwrap();
        fs::write(dir.join("native").join(triple).join("libx.bin"), triple).unwrap();
    }
    publish(&dir, None).unwrap();
    let stored = names(&fake.archive("withnative", "1.0.0").unwrap());
    assert!(stored.contains(&"native/x86_64-unknown-linux-gnu/libx.bin".to_string()) && stored.contains(&"native/aarch64-apple-darwin/libx.bin".to_string()), "{stored:?}");
}

#[test]
fn a_checksum_the_registry_answers_wrongly_is_an_error() {
    let _turn = turn(Some(TOKEN));
    let fake = Fake::start();
    fake.state.lock().unwrap().wrong_checksum = true;
    let err = publish(&project(&fake, "wrongsum", META), None).unwrap_err();
    assert!(err.contains("stored sha256:00"), "{err}");
}

#[test]
fn an_unknown_registry_name_is_an_error() {
    let _turn = turn(Some(TOKEN));
    let fake = Fake::start();
    let err = publish(&project(&fake, "unknownreg", META), Some("nowhere")).unwrap_err();
    assert!(err.contains("no registry named `nowhere`"), "{err}");
}
