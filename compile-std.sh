#!/usr/bin/env bash
# Recompiles C/Rust libraries in mote_std/ (all, or the named library [name]),
# automatically selecting zig (lib/build.sh) or rust (rust/Cargo.toml).
# Usage: ./compile-std.sh [--check] [name]
set -eu
root=$(cd "$(dirname "$0")" && pwd)
exec "$root/tools/build-std-libs.sh" --core "$root/mote_std" "$@"
