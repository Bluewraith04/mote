//! A `for` over a struct, class or enum is a compile error, not a runtime panic.

use std::process::Command;

#[test]
fn looping_over_a_class_names_what_to_loop_over() {
    let source = "class Counter {\n    var n: Int\n    pub fn next(var self) -> Int? { return None }\n}\n\nfn main() {\n    var c = Counter { n: 0 }\n    for v in c {\n        println(v)\n    }\n}\n";
    let dir = std::env::temp_dir().join(format!("mote_for_user_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.mote");
    std::fs::write(&file, source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("check").arg(&file).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(!out.status.success());
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(text.contains("cannot loop over a `Counter`"), "{text}");
}
