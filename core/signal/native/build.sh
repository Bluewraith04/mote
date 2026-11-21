#!/usr/bin/env bash
# Builds libsignal.so and libsignal.a into $MOTE_NATIVE_OUT.
set -eu
here=$(dirname "$0")
cache=$(mktemp -d)
trap 'rm -rf "$cache"' EXIT
zig build-lib "$here/signal.zig" -dynamic -OReleaseSafe -fPIC -fstrip -fno-stack-check --cache-dir "$cache" --global-cache-dir "$cache" -femit-bin="$MOTE_NATIVE_OUT/libsignal.so" -fno-emit-h
zig build-lib "$here/signal.zig" -static -OReleaseSafe -fPIC -fstrip -fno-stack-check -fno-compiler-rt --cache-dir "$cache" --global-cache-dir "$cache" -femit-bin="$MOTE_NATIVE_OUT/libsignal.a" -fno-emit-h
