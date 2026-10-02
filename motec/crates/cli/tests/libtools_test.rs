//! `std.dev.libtools` opens a C library and calls its functions through `bind<F>`.

use std::path::{Path, PathBuf};
use std::process::Command;

const C_SOURCE: &str = "\
#include <stdint.h>
#include <unistd.h>
int64_t add_i(int64_t a, int64_t b) { return a + b; }
double scale(double x, double k) { return x * k; }
int negate(int b) { return !b; }
const char *greeting(void) { return \"hello from C\"; }
int64_t length(const char *s) { int64_t n = 0; while (s[n]) n++; return n; }
int64_t fill(char *buf, int64_t n) { for (int64_t i = 0; i < n; i++) buf[i] = (char)(65 + i); return n; }
static int64_t count = 0;
void bump(void) { count++; }
int64_t total(void) { return count; }
int64_t nap(int64_t ms) { usleep(ms * 1000); return ms; }
int64_t six(int64_t a, int64_t b, int64_t c, int64_t d, int64_t e, int64_t f) { return a + b + c + d + e + f; }
";

fn have_cc() -> bool {
    Command::new("cc").arg("--version").output().is_ok_and(|o| o.status.success())
}

fn build_lib(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mote_libtools_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("lib.c");
    std::fs::write(&src, C_SOURCE).unwrap();
    let out = dir.join("libmotetest.so");
    let status = Command::new("cc").arg("-shared").arg("-fPIC").arg("-o").arg(&out).arg(&src).status().unwrap();
    assert!(status.success(), "cc failed");
    out
}

fn mote(verb: &str, body: &str, tag: &str, extra: &[&str], env: &[(&str, &str)]) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_libtools_run_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.mote");
    let source = format!("import {{ open, bind, bind_blocking }} from std.dev.libtools\n\nfn main() {{\n{body}\n}}\n");
    std::fs::write(&file, source).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mote"));
    cmd.arg(verb).arg(&file).args(extra);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn run(body: &str, tag: &str) -> String {
    let (ok, text) = mote("run", body, tag, &["--allow-native"], &[]);
    assert!(ok, "{text}");
    text
}

fn lib_literal(lib: &Path) -> String {
    lib.to_string_lossy().replace('\\', "\\\\")
}

#[test]
#[cfg(target_os = "linux")]
fn test_libm_functions_are_called_through_bind() {
    let out = run(
        "    let libm = open(\"libm.so.6\")?\n    let sin = bind<(Float) -> Float>(libm, \"sin\")?\n    let pow = bind<(Float, Float) -> Float>(libm, \"pow\")?\n    println(sin(0.0))\n    println(pow(2.0, 10.0))",
        "libm",
    );
    assert_eq!(out, "0.0\n1024.0\n");
}

#[test]
#[cfg(target_os = "linux")]
fn test_a_bound_function_can_move_into_a_spawn() {
    let out = run(
        "    let libm = open(\"libm.so.6\")?\n    let sqrt = bind<(Float) -> Float>(libm, \"sqrt\")?\n    let t = spawn { sqrt(16.0) }\n    println(t.join()?)",
        "spawn",
    );
    assert_eq!(out, "4.0\n");
}

#[test]
fn test_c_values_cross_in_both_directions() {
    if !have_cc() {
        return;
    }
    let lib = lib_literal(&build_lib("values"));
    let body = format!(
        "    let lib = open(\"{lib}\")?\n\
         \x20   let add = bind<(Int, Int) -> Int>(lib, \"add_i\")?\n\
         \x20   let scale = bind<(Float, Float) -> Float>(lib, \"scale\")?\n\
         \x20   let negate = bind<(Bool) -> Bool>(lib, \"negate\")?\n\
         \x20   let greeting = bind<() -> String>(lib, \"greeting\")?\n\
         \x20   let length = bind<(String) -> Int>(lib, \"length\")?\n\
         \x20   let six = bind<(Int, Int, Int, Int, Int, Int) -> Int>(lib, \"six\")?\n\
         \x20   println(add(40, 2))\n\
         \x20   println(scale(1.5, 4.0))\n\
         \x20   println(negate(true))\n\
         \x20   println(greeting())\n\
         \x20   println(length(\"mote\"))\n\
         \x20   println(six(1, 2, 3, 4, 5, 6))"
    );
    assert_eq!(run(&body, "values"), "42\n6.0\nfalse\nhello from C\n4\n21\n");
}

#[test]
fn test_a_void_function_and_its_state() {
    if !have_cc() {
        return;
    }
    let lib = lib_literal(&build_lib("void"));
    let body = format!(
        "    let lib = open(\"{lib}\")?\n\
         \x20   let bump = bind<() -> Null>(lib, \"bump\")?\n\
         \x20   let total = bind<() -> Int>(lib, \"total\")?\n\
         \x20   bump()\n\
         \x20   bump()\n\
         \x20   println(total())\n\
         \x20   lib.close()\n\
         \x20   bump()\n\
         \x20   println(total())"
    );
    assert_eq!(run(&body, "void"), "2\n3\n");
}

#[test]
fn test_a_buffer_is_filled_by_c() {
    if !have_cc() {
        return;
    }
    let lib = lib_literal(&build_lib("buffer"));
    let body = format!(
        "    let lib = open(\"{lib}\")?\n\
         \x20   let fill = bind<(Bytes, Int) -> Int>(lib, \"fill\")?\n\
         \x20   let buf = Bytes(8)\n\
         \x20   println(fill(buf, 4))\n\
         \x20   println(buf[0])\n\
         \x20   println(buf[3])"
    );
    assert_eq!(run(&body, "buffer"), "4\n65\n68\n");
}

#[test]
fn test_a_slow_c_call_does_not_hold_the_worker() {
    if !have_cc() {
        return;
    }
    let lib = lib_literal(&build_lib("nap"));
    let body = format!(
        "    let lib = open(\"{lib}\")?\n\
         \x20   let nap = bind_blocking<(Int) -> Int>(lib, \"nap\")?\n\
         \x20   let slow = spawn {{\n        nap(400)\n        println(\"napped\")\n    }}\n\
         \x20   let quick = spawn {{ println(\"other\") }}\n\
         \x20   slow.join()?\n\
         \x20   quick.join()?"
    );
    let (ok, text) = mote("run", &body, "nap", &["--allow-native", "--workers", "1"], &[]);
    assert!(ok, "{text}");
    assert_eq!(text, "other\nnapped\n");
}

#[test]
fn test_an_inline_c_call_holds_the_worker_until_it_returns() {
    if !have_cc() {
        return;
    }
    let lib = lib_literal(&build_lib("nap_inline"));
    let body = format!(
        "    let lib = open(\"{lib}\")?\n\
         \x20   let nap = bind<(Int) -> Int>(lib, \"nap\")?\n\
         \x20   let slow = spawn {{\n        nap(200)\n        println(\"napped\")\n    }}\n\
         \x20   let quick = spawn {{ println(\"other\") }}\n\
         \x20   slow.join()?\n\
         \x20   quick.join()?"
    );
    let (ok, text) = mote("run", &body, "nap_inline", &["--allow-native", "--workers", "1"], &[]);
    assert!(ok, "{text}");
    assert_eq!(text, "napped\nother\n");
}

#[test]
#[cfg(target_os = "linux")]
fn test_bind_blocking_answers_like_bind() {
    let out = run(
        "    let libm = open(\"libm.so.6\")?\n    let pow = bind_blocking<(Float, Float) -> Float>(libm, \"pow\")?\n    let t = spawn { pow(2.0, 8.0) }\n    println(pow(2.0, 10.0))\n    println(t.join()?)",
        "blocking_pow",
    );
    assert_eq!(out, "1024.0\n256.0\n");
}

#[test]
fn test_a_task_cancelled_inside_a_c_call_stops_after_it_returns() {
    if !have_cc() {
        return;
    }
    let lib = lib_literal(&build_lib("nap_cancel"));
    let body = format!(
        "    let lib = open(\"{lib}\")?\n\
         \x20   let nap = bind_blocking<(Int) -> Int>(lib, \"nap\")?\n\
         \x20   let t = spawn {{ nap(300) }}\n\
         \x20   nap(100)\n\
         \x20   t.cancel()\n\
         \x20   match t.join() {{\n        Ok(_) => {{ println(\"finished\") }}\n        Err(_) => {{ println(\"cancelled\") }}\n    }}"
    );
    let (ok, text) = mote("run", &body, "nap_cancel", &["--allow-native", "--workers", "1"], &[]);
    assert!(ok, "{text}");
    assert!(text == "cancelled\n" || text == "finished\n", "{text}");
}

#[test]
fn test_open_needs_the_flag() {
    let body = "    match open(\"libm.so.6\") {\n        Ok(_) => { println(\"opened\") }\n        Err(e) => { println(e.message) }\n    }";
    let (ok, text) = mote("run", body, "refused", &[], &[]);
    assert!(ok, "{text}");
    assert_eq!(text, "native libraries are off; run with --allow-native\n");
}

#[test]
#[cfg(target_os = "linux")]
fn test_the_environment_can_allow_it() {
    let body = "    match open(\"libm.so.6\") {\n        Ok(_) => { println(\"opened\") }\n        Err(e) => { println(e.message) }\n    }";
    let (ok, text) = mote("run", body, "env", &[], &[("MOTE_ALLOW_NATIVE", "1")]);
    assert!(ok, "{text}");
    assert_eq!(text, "opened\n");
}

#[test]
fn test_a_missing_library_and_a_missing_symbol_are_errors() {
    if !have_cc() {
        return;
    }
    let lib = lib_literal(&build_lib("missing"));
    let body = format!(
        "    match open(\"/no/such/libnothing.so\") {{\n        Ok(_) => {{ println(\"opened\") }}\n        Err(e) => {{ println(\"no library\") }}\n    }}\n\
         \x20   let lib = open(\"{lib}\")?\n\
         \x20   match bind<() -> Int>(lib, \"nothing_here\") {{\n        Ok(_) => {{ println(\"bound\") }}\n        Err(e) => {{ println(\"no symbol\") }}\n    }}"
    );
    assert_eq!(run(&body, "missing"), "no library\nno symbol\n");
}

#[test]
fn test_a_type_c_cannot_take_is_a_compile_error() {
    let (ok, text) = mote(
        "check",
        "    let lib = open(\"x\")?\n    let f = bind<(List<Int>) -> Int>(lib, \"f\")?\n    println(f([1]))",
        "bad_param",
        &[],
        &[],
    );
    assert!(!ok);
    assert!(text.contains("a C function takes Int, Float, Bool, String or Bytes"), "{text}");
}

#[test]
fn test_bind_without_a_function_type_is_a_compile_error() {
    let (ok, text) = mote("check", "    let lib = open(\"x\")?\n    let f = bind<Int>(lib, \"f\")?\n    println(f)", "not_fn", &[], &[]);
    assert!(!ok);
    assert!(text.contains("`bind` needs a function type"), "{text}");
}

#[test]
fn test_a_result_type_c_cannot_return_is_a_compile_error() {
    let (ok, text) = mote("check", "    let lib = open(\"x\")?\n    let f = bind<() -> List<Int>>(lib, \"f\")?\n    println(f())", "bad_ret", &[], &[]);
    assert!(!ok);
    assert!(text.contains("a C function returns Int, Float, Bool, String or Null"), "{text}");
}
