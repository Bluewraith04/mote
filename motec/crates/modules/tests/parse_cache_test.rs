//! The on-disk parse cache.

use std::fs;
use std::path::{Path, PathBuf};
use modules::MultiFileCompiler;

fn project(name: &str, helper: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mote_parse_cache_{}_{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("helper.mote"), helper).unwrap();
    fs::write(dir.join("main.mote"), "import .helper\nfn main() -> Int {\n    return compute(5) + 2\n}\nreturn main()").unwrap();
    dir
}

fn compile(dir: &Path, cached: bool) -> Result<Vec<u8>, String> {
    let mut compiler = MultiFileCompiler::new(dir.to_path_buf());
    if cached {
        compiler = compiler.with_cache(dir.join(".build"));
    }
    compiler.compile_program(&dir.join("main.mote")).map(|p| p.to_bytes())
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for shard in fs::read_dir(dir.join(".build/modules")).unwrap().flatten() {
        out.extend(fs::read_dir(shard.path()).unwrap().flatten().map(|e| e.path()));
    }
    out.sort();
    out
}

#[test]
fn a_cache_hit_compiles_to_the_same_bytes() {
    let dir = project("hit", "pub fn compute(x: Int) -> Int { return x * 10 }");
    let plain = compile(&dir, false).unwrap();
    let cold = compile(&dir, true).unwrap();
    let written = entries(&dir);
    assert!(written.len() >= 2, "main and helper are cached");
    let warm = compile(&dir, true).unwrap();
    assert_eq!(plain, cold);
    assert_eq!(plain, warm);
    assert_eq!(entries(&dir), written, "a hit writes nothing new");
    fs::remove_dir_all(dir).ok();
}

#[test]
fn a_corrupt_entry_is_reparsed_and_rewritten() {
    let dir = project("corrupt", "pub fn compute(x: Int) -> Int { return x * 10 }");
    let cold = compile(&dir, true).unwrap();
    for entry in entries(&dir) {
        fs::write(entry, b"\x01\x00\x00\x00garbage").unwrap();
    }
    assert_eq!(compile(&dir, true).unwrap(), cold);
    assert!(entries(&dir).iter().all(|e| fs::metadata(e).unwrap().len() > 11), "entries rewritten");
    assert_eq!(compile(&dir, true).unwrap(), cold);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn an_edited_file_gets_a_new_entry() {
    let dir = project("edit", "pub fn compute(x: Int) -> Int { return x * 10 }");
    compile(&dir, true).unwrap();
    let before = entries(&dir).len();
    fs::write(dir.join("helper.mote"), "pub fn compute(x: Int) -> Int { return x * 20 }").unwrap();
    assert_eq!(compile(&dir, true).unwrap(), compile(&dir, false).unwrap());
    assert_eq!(entries(&dir).len(), before + 1);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn diagnostics_on_a_hit_quote_the_right_source() {
    let dir = project("diag", "pub fn compute(x: Int) -> Int {\n    let s: String = x\n    return x\n}");
    let plain = compile(&dir, false).unwrap_err();
    let cold = compile(&dir, true).unwrap_err();
    let warm = compile(&dir, true).unwrap_err();
    assert!(plain.contains("let s: String = x"), "{plain}");
    assert_eq!(plain, cold);
    assert_eq!(plain, warm);
    fs::remove_dir_all(dir).ok();
}
