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
int64_t reverse(const char *in, int64_t n, char *out, int64_t cap) { if (n > cap) return n; for (int64_t i = 0; i < n; i++) out[i] = in[n - 1 - i]; return n; }
int64_t slow_fill(char *out, int64_t n) { usleep(300000); for (int64_t i = 0; i < n; i++) out[i] = (char)(97 + i % 26); return n; }
int64_t reject(const char *in, int64_t n, char *out, int64_t cap) { const char *m = \"bad input\"; for (int64_t i = 0; i < 9 && i < cap; i++) out[i] = m[i]; return -1; }
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

fn bytes_convention_body(lib: &str, bind: &str) -> String {
    format!(
        "    let lib = open(\"{lib}\")?\n\
         \x20   let reverse = {bind}<(Bytes, Int, Bytes, Int) -> Int>(lib, \"reverse\")?\n\
         \x20   let reject = {bind}<(Bytes, Int, Bytes, Int) -> Int>(lib, \"reject\")?\n\
         \x20   let input = \"hello\".bytes()\n\
         \x20   var out = Bytes(2)\n\
         \x20   var n = reverse(input, input.len(), out, out.len())\n\
         \x20   println(n)\n\
         \x20   if n > out.len() {{\n\
         \x20       out = Bytes(n)\n\
         \x20       n = reverse(input, input.len(), out, out.len())\n\
         \x20   }}\n\
         \x20   println(out.slice(0, n).decode())\n\
         \x20   let empty = Bytes()\n\
         \x20   println(reverse(empty, 0, out, out.len()))\n\
         \x20   let msg = Bytes(32)\n\
         \x20   let code = reject(input, input.len(), msg, msg.len())\n\
         \x20   println(code)\n\
         \x20   println(msg.slice(0, 9).decode())"
    )
}

#[test]
fn test_bytes_in_and_bytes_out_need_no_pointer_type() {
    if !have_cc() {
        return;
    }
    let lib = lib_literal(&build_lib("bytes_convention"));
    let out = run(&bytes_convention_body(&lib, "bind"), "bytes_convention");
    assert_eq!(out, "5\nolleh\n0\n-1\nbad input\n");
}

#[test]
fn test_the_bytes_convention_holds_on_a_helper_thread() {
    if !have_cc() {
        return;
    }
    let lib = lib_literal(&build_lib("bytes_blocking"));
    let out = run(&bytes_convention_body(&lib, "bind_blocking"), "bytes_blocking");
    assert_eq!(out, "5\nolleh\n0\n-1\nbad input\n");
}

#[test]
fn test_a_buffer_survives_collections_while_c_writes_to_it() {
    if !have_cc() {
        return;
    }
    let lib = lib_literal(&build_lib("bytes_gc"));
    let body = format!(
        "    let lib = open(\"{lib}\")?\n\
         \x20   let slow_fill = bind_blocking<(Bytes, Int) -> Int>(lib, \"slow_fill\")?\n\
         \x20   let churn = spawn {{\n\
         \x20       var total = 0\n\
         \x20       for i in 0..200000 {{\n\
         \x20           let xs = [i, i + 1, i + 2]\n\
         \x20           total += xs.len()\n\
         \x20       }}\n\
         \x20       total\n\
         \x20   }}\n\
         \x20   let buf = Bytes(40)\n\
         \x20   println(slow_fill(buf, 40))\n\
         \x20   println(buf[0])\n\
         \x20   println(buf[27])\n\
         \x20   println(churn.join()?)"
    );
    let (ok, text) = mote("run", &body, "bytes_gc", &["--allow-native", "--workers", "1", "--mem-stats"], &[]);
    assert!(ok, "{text}");
    assert!(text.starts_with("40\n97\n98\n600000\n"), "{text}");
    assert!(text.contains("mem.gc.collections = ") && !text.contains("mem.gc.collections = 0\n"), "{text}");
}

/// A package `app` with a granted path dependency `cl` carrying `libmotecl` for this machine; `main` is the body of `fn main()`.
fn native_app(tag: &str, main: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("mote_libtools_pkg_{tag}_{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();
    let native = root.join("app/cl/native").join(pkg::native::host_triple());
    std::fs::create_dir_all(&native).unwrap();
    std::fs::create_dir_all(root.join("app/cl/src")).unwrap();
    std::fs::create_dir_all(root.join("app/src")).unwrap();
    let lib = build_lib(tag);
    let name = format!("{}motecl{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX);
    std::fs::copy(&lib, native.join(name)).unwrap();
    std::fs::remove_dir_all(lib.parent().unwrap()).ok();
    std::fs::write(root.join("app/cl/mote.toml"), "[package]\nname = \"cl\"\nversion = \"1.0.0\"\n").unwrap();
    std::fs::write(root.join("app/cl/src/lib.mote"), "pub fn v() -> Int { return 1 }\n").unwrap();
    std::fs::write(
        root.join("app/mote.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\ncl = { path = \"cl\", native = true }\n",
    )
    .unwrap();
    let source = format!("import {{ open_package, bind }} from std.dev.libtools\n\nfn main() {{\n{main}\n}}\n");
    std::fs::write(root.join("app/src/main.mote"), source).unwrap();
    root.join("app")
}

fn mote_in(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).args(args).current_dir(dir).output().unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

const ADD_FROM_PACKAGE: &str = "    match open_package(\"cl\", \"motecl\") {\n        Ok(lib) => {\n            let add = bind<(Int, Int) -> Int>(lib, \"add_i\")?\n            println(add(40, 2))\n        }\n        Err(e) => { println(e.message) }\n    }";

#[test]
fn test_a_granted_package_opens_its_library_without_the_flag() {
    if !have_cc() {
        return;
    }
    let app = native_app("pkg_run", ADD_FROM_PACKAGE);
    let (ok, text) = mote_in(&app, &["install"]);
    assert!(ok, "{text}");
    let (ok, text) = mote_in(&app, &["run"]);
    assert!(ok, "{text}");
    assert_eq!(text, "42\n");
    std::fs::remove_dir_all(app.parent().unwrap()).ok();
}

#[test]
fn test_a_built_program_finds_its_libraries_in_the_lib_directory() {
    if !have_cc() {
        return;
    }
    let app = native_app("pkg_build", ADD_FROM_PACKAGE);
    let (ok, text) = mote_in(&app, &["install"]);
    assert!(ok, "{text}");
    let (ok, text) = mote_in(&app, &["build"]);
    assert!(ok, "{text}");
    let lib_file = format!("{}motecl{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX);
    assert!(app.join("dist/app.lib/cl").join(&lib_file).is_file(), "{text}");

    let exe = app.join("dist").join(format!("app{}", std::env::consts::EXE_SUFFIX));
    let elsewhere = std::env::temp_dir();
    let out = Command::new(&exe).current_dir(&elsewhere).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "42\n");

    std::fs::remove_dir_all(app.join("dist/app.lib")).unwrap();
    let out = Command::new(&exe).current_dir(&elsewhere).output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(!text.contains("42"), "a program without its .lib directory cannot open the library: {text}");
    assert!(text.contains("cl is not granted native access"), "{text}");
    std::fs::remove_dir_all(app.parent().unwrap()).ok();
}

#[test]
fn test_a_package_that_is_not_granted_cannot_open_a_library() {
    let app = native_app("pkg_denied", "    match open_package(\"other\", \"motecl\") {\n        Ok(_) => { println(\"opened\") }\n        Err(e) => { println(e.message) }\n    }");
    let (ok, text) = mote_in(&app, &["install"]);
    assert!(ok, "{text}");
    let (ok, text) = mote_in(&app, &["run"]);
    assert!(ok, "{text}");
    assert_eq!(text, "other is not granted native access\n");
    std::fs::remove_dir_all(app.parent().unwrap()).ok();
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
