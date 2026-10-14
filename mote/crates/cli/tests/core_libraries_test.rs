//! Core libraries are found by plain name in `MOTE_CORE_PATH`, after the program's own modules.

use std::fs;
use std::path::Path;
use std::process::Command;

fn run(dir: &Path, core: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_mote"))
        .env("MOTE_CORE_PATH", core)
        .arg("run")
        .arg(dir.join("main.mote"))
        .output()
        .unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mote_core_{tag}_{}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(dir.join("core/shapes/src")).unwrap();
    fs::write(dir.join("core/shapes/src/lib.mote"), "pub fn sides() -> Int {\n    return 4\n}\n").unwrap();
    fs::write(dir.join("core/shapes/src/extra.mote"), "pub fn more() -> Int {\n    return 5\n}\n").unwrap();
    dir
}

#[test]
fn a_core_library_is_imported_by_its_name() {
    let dir = scratch("name");
    fs::write(dir.join("main.mote"), "import shapes\nimport shapes.extra as extra\n\nfn main() {\n    println(shapes.sides() + extra.more())\n}\n").unwrap();
    let (ok, text) = run(&dir, &dir.join("core"));
    fs::remove_dir_all(&dir).ok();
    assert!(ok && text == "9\n", "got: {text}");
}

#[test]
fn a_module_of_the_program_wins_over_a_core_library() {
    let dir = scratch("own");
    fs::write(dir.join("shapes.mote"), "pub fn sides() -> Int {\n    return 3\n}\n").unwrap();
    fs::write(dir.join("main.mote"), "import shapes\n\nfn main() {\n    println(shapes.sides())\n}\n").unwrap();
    let (ok, text) = run(&dir, &dir.join("core"));
    fs::remove_dir_all(&dir).ok();
    assert!(ok && text == "3\n", "got: {text}");
}
