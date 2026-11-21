#!/usr/bin/env bash
# Builds libenv.so and libenv.a into $MOTE_NATIVE_OUT.
set -eu
here=$(dirname "$0")
cache=$(mktemp -d)
trap 'rm -rf "$cache"' EXIT
zig build-lib "$here/env.zig" -dynamic -OReleaseSafe -fPIC -fstrip -fno-stack-check -fsingle-threaded --cache-dir "$cache" --global-cache-dir "$cache" -femit-bin="$MOTE_NATIVE_OUT/libenv.so" -fno-emit-h
zig build-lib "$here/env.zig" -static -OReleaseSafe -fPIC -fstrip -fno-stack-check -fsingle-threaded -fno-compiler-rt --cache-dir "$cache" --global-cache-dir "$cache" -femit-bin="$MOTE_NATIVE_OUT/libenv.a" -fno-emit-h
