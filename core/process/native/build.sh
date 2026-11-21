#!/usr/bin/env bash
# Builds libprocess.so and libprocess.a into $MOTE_NATIVE_OUT.
set -eu
here=$(dirname "$0")
cache=$(mktemp -d)
trap 'rm -rf "$cache"' EXIT
zig build-lib "$here/process.zig" -dynamic -OReleaseSafe -fPIC -fstrip -fno-stack-check -fsingle-threaded --cache-dir "$cache" --global-cache-dir "$cache" -femit-bin="$MOTE_NATIVE_OUT/libprocess.so" -fno-emit-h
zig build-lib "$here/process.zig" -static -OReleaseSafe -fPIC -fstrip -fno-stack-check -fsingle-threaded -fno-compiler-rt --cache-dir "$cache" --global-cache-dir "$cache" -femit-bin="$MOTE_NATIVE_OUT/libprocess.a" -fno-emit-h
