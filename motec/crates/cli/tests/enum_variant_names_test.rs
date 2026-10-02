//! Two enums may share a variant name, including the prelude `Result`'s `Ok` and `Err`.

use std::process::Command;

fn run(source: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_variants_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.mote");
    std::fs::write(&file, source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(&file).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn an_enum_may_name_a_variant_ok() {
    let source = "enum Reply { Ok(Int), NotFound }\n\nfn show(r: Reply) -> String {\n    match r {\n        Ok(n) => { return \"ok ${n}\" }\n        NotFound => { return \"missing\" }\n    }\n}\n\nfn main() {\n    println(show(Reply.Ok(1)))\n    println(show(Reply.NotFound))\n    let r: Result<Int, String> = Err(\"bad\")\n    match r {\n        Ok(n) => println(n)\n        Err(e) => println(e)\n    }\n}\n";
    assert_eq!(run(source, "ok"), "ok 1\nmissing\nbad\n");
}

#[test]
fn a_bare_unit_variant_in_a_pattern_uses_the_subjects_enum() {
    let source = "enum A { Empty, Full(Int) }\nenum B { Empty, Full(Int) }\n\nfn name(a: A) -> String {\n    match a {\n        Empty => { return \"A empty\" }\n        Full(n) => { return \"A full ${n}\" }\n    }\n}\n\nfn other(b: B) -> String {\n    match b {\n        Empty => { return \"B empty\" }\n        Full(n) => { return \"B full ${n}\" }\n    }\n}\n\nfn main() {\n    println(name(A.Empty))\n    println(name(A.Full(3)))\n    println(other(B.Empty))\n    println(other(B.Full(4)))\n}\n";
    assert_eq!(run(source, "unit"), "A empty\nA full 3\nB empty\nB full 4\n");
}
