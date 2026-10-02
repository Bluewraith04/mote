//! A module-level `var b = a` of a struct copies it.

use std::process::Command;

#[test]
fn a_top_level_struct_binding_is_a_copy() {
    let source = "struct Point {\n    var x: Int\n    var y: Int\n}\n\nvar a = Point { x: 1, y: 2 }\nvar b = a\nb.x = 99\nprintln(a.x)\nprintln(b.x)\n";
    let dir = std::env::temp_dir().join(format!("mote_top_copy_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.mote");
    std::fs::write(&file, source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(&file).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "1\n99\n");
}
