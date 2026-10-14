//! The global registry config and the registries a project names.

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use pkg::PackageManager;

/// Points the mote home at a directory of this test run.
fn home() -> MutexGuard<'static, ()> {
    static TURN: Mutex<()> = Mutex::new(());
    static HOME: std::sync::Once = std::sync::Once::new();
    let guard = TURN.lock().unwrap_or_else(|e| e.into_inner());
    HOME.call_once(|| std::env::set_var("MOTE_HOME", std::env::temp_dir().join(format!("mote_home_registry_config_{}", std::process::id()))));
    fs::remove_file(PathBuf::from(std::env::var("MOTE_HOME").unwrap()).join("mote_config.toml")).ok();
    guard
}

fn project(name: &str, extra: &str) -> PathBuf {
    let app = std::env::temp_dir().join(format!("mote_registry_{name}_{}", std::process::id()));
    fs::remove_dir_all(&app).ok();
    fs::create_dir_all(&app).unwrap();
    fs::write(app.join("mote.toml"), format!("[package]\nname = \"app\"\nversion = \"0.1.0\"\n{extra}")).unwrap();
    app
}

#[test]
fn a_bare_version_with_no_registry_says_how_to_add_one() {
    let _home = home();
    let app = project("none", "\n[dependencies]\nshapes = \"^1.0\"\n");
    let err = PackageManager::install_dependencies(&app, false).unwrap_err();
    assert!(err.contains("mote package --add-registry NAME URL"), "{err}");
}

#[test]
fn registries_are_named_in_the_global_config_with_one_default() {
    let _home = home();
    pkg::config::add_registry("first", "https://one.example.com/", None, false).unwrap();
    pkg::config::add_registry("second", "http://localhost:9000", None, false).unwrap();
    let config = pkg::config::Config::load().unwrap();
    assert_eq!(config.default.as_deref(), Some("first"));
    assert_eq!(config.registries["first"].url, "https://one.example.com");

    pkg::config::add_registry("second", "http://localhost:9000", None, true).unwrap();
    assert_eq!(pkg::config::Config::load().unwrap().default.as_deref(), Some("second"));
    assert!(pkg::config::add_registry("plain", "http://example.com", None, false).unwrap_err().contains("https"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let file = PathBuf::from(std::env::var("MOTE_HOME").unwrap()).join("mote_config.toml");
        assert_eq!(fs::metadata(file).unwrap().permissions().mode() & 0o777, 0o600);
    }

    pkg::config::remove_registry("first", None).unwrap();
    pkg::config::remove_registry("second", None).unwrap();
    assert!(pkg::config::remove_registry("second", None).is_err());
    assert_eq!(pkg::config::Config::load().unwrap().default, None);
}

#[test]
fn a_token_is_kept_when_the_url_stays_and_dropped_when_it_changes() {
    let _home = home();
    let file = PathBuf::from(std::env::var("MOTE_HOME").unwrap()).join("mote_config.toml");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "default = \"main\"\n\n[registries.main]\nurl = \"https://reg.example.com\"\ntoken = \"secret\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(pkg::config::Config::load().unwrap_err().contains("chmod 600"));
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    }
    pkg::config::add_registry("main", "https://reg.example.com/", None, false).unwrap();
    assert_eq!(pkg::config::Config::load().unwrap().registries["main"].token.as_deref(), Some("secret"));
    pkg::config::add_registry("main", "https://elsewhere.example.com", None, false).unwrap();
    assert_eq!(pkg::config::Config::load().unwrap().registries["main"].token, None);
}

#[test]
fn registries_can_be_named_in_the_project() {
    let _home = home();
    let app = project("local", "");
    pkg::config::add_registry("team", "https://reg.example.com", Some(&app), false).unwrap();
    let text = fs::read_to_string(app.join("mote.toml")).unwrap();
    assert!(text.contains("team = \"https://reg.example.com\""), "{text}");
    pkg::config::remove_registry("team", Some(&app)).unwrap();
    assert!(!fs::read_to_string(app.join("mote.toml")).unwrap().contains("team"));
}
