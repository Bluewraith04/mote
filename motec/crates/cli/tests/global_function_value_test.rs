//! A module-level `let` holding a function value can be called.

use std::process::Command;

#[test]
fn a_global_function_value_is_called_at_the_top_level_and_in_a_function() {
    let source = "let add: (Int, Int) -> Int = |a, b| a + b\nprintln(add(4, 5))\n\nfn counter() -> () -> Int {\n    var n = 0\n    return || {\n        n += 1\n        return n\n    }\n}\nlet next = counter()\nprintln(next())\nprintln(next())\n\nfn show() {\n    println(add(1, 1))\n    println(next())\n}\nshow()\n";
    let dir = std::env::temp_dir().join(format!("mote_global_fn_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.mote");
    std::fs::write(&file, source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(&file).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "9\n1\n2\n2\n3\n");
}
