//! Builds `libmote_runtime.a`, the runtime an executable links, and leaves it in `OUT_DIR` for the toolchain to carry.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets the manifest directory"));
    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets the output directory"));
    for dir in ["runtime", "abi", "heap", "scheduler", "base"] {
        println!("cargo:rerun-if-changed=../{dir}/src");
        println!("cargo:rerun-if-changed=../{dir}/Cargo.toml");
    }
    println!("cargo:rerun-if-changed=build.rs");
    let target = out.join("runtime-target");
    let status = Command::new(env::var("CARGO").expect("cargo sets its own path"))
        .args(["build", "--release", "--package", "mote-runtime", "--features", "executable", "--manifest-path"])
        .arg(manifest.join("../Cargo.toml"))
        .env("CARGO_TARGET_DIR", &target)
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("RUSTC_WRAPPER")
        .status()
        .expect("cargo runs");
    assert!(status.success(), "the runtime library is built");
    std::fs::copy(target.join("release/libmote_runtime.a"), out.join("libmote_runtime.a")).expect("the runtime library is where cargo puts it");
}
