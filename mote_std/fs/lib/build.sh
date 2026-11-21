#!/usr/bin/env bash
# Builds libfs.so and libfs.a into $MOTE_LIB_OUT.
set -eu
here=$(dirname "$0")
cache=$(mktemp -d)
trap 'rm -rf "$cache"' EXIT
zig build-lib "$here/fs.zig" -dynamic -OReleaseSafe -fPIC -fstrip -fno-stack-check -fsingle-threaded --cache-dir "$cache" --global-cache-dir "$cache" -femit-bin="$MOTE_LIB_OUT/libfs.so" -fno-emit-h
zig build-lib "$here/fs.zig" -static -OReleaseSafe -fPIC -fstrip -fno-stack-check -fsingle-threaded -fno-compiler-rt --cache-dir "$cache" --global-cache-dir "$cache" -femit-bin="$MOTE_LIB_OUT/libfs.a" -fno-emit-h
