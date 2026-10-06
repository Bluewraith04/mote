//! An error inside `${…}` points at the hole's own place in the file.

use std::process::Command;

#[test]
fn an_error_in_a_hole_points_into_the_string() {
    let dir = std::env::temp_dir().join(format!("mote_hole_span_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), "pub fn main() {\n    let x = 5\n    println(\"v ${x.nope}\")\n}\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("check").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success() && text.contains("main.mote:3:18"), "{text}");
}
