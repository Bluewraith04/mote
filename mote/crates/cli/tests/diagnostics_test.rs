//! Diagnostics name what the user wrote, not the module-mangled symbol.

use std::process::Command;

#[test]
fn an_imported_function_is_named_without_its_module_prefix() {
    let dir = std::env::temp_dir().join(format!("mote_diag_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("math.mote"), "pub fn add(a: Int, b: Int) -> Int {\n    return a + b\n}\n").unwrap();
    std::fs::write(dir.join("main.mote"), "import { add } from .math\nadd(1)\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("`add` takes 2 argument(s)"), "got: {err}");
    assert!(!err.contains("_m"), "got: {err}");
}
