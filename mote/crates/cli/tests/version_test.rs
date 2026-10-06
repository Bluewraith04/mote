//! `mote --version` and `mote --help` spellings.

use std::process::Command;

fn mote(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).args(args).output().unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn every_spelling_prints_the_version() {
    for flag in ["--version", "-V", "version"] {
        let (ok, stdout) = mote(&[flag]);
        assert!(ok, "{flag}");
        assert_eq!(stdout, format!("mote {}\n", env!("CARGO_PKG_VERSION")), "{flag}");
    }
}

#[test]
fn no_arguments_and_help_print_usage() {
    for args in [&[][..], &["--help"], &["-h"], &["help"]] {
        let (ok, stdout) = mote(args);
        assert!(ok, "{args:?}");
        assert!(stdout.starts_with("Usage: mote"), "{args:?}: {stdout}");
    }
}
