#!/usr/bin/env bash
# Builds libnet.so and libnet.a (sockets, Zig) and libnettls.so and libnettls.a (TLS, Rust) into $MOTE_NATIVE_OUT.
set -eu
here=$(dirname "$0")
cache=$(mktemp -d)
trap 'rm -rf "$cache"' EXIT
zig build-lib "$here/net.zig" -dynamic -OReleaseSafe -fPIC -fstrip -fno-stack-check -fsingle-threaded --cache-dir "$cache" --global-cache-dir "$cache" -femit-bin="$MOTE_NATIVE_OUT/libnet.so" -fno-emit-h
zig build-lib "$here/net.zig" -static -OReleaseSafe -fPIC -fstrip -fno-stack-check -fsingle-threaded -fno-compiler-rt --cache-dir "$cache" --global-cache-dir "$cache" -femit-bin="$MOTE_NATIVE_OUT/libnet.a" -fno-emit-h
cargo build --release --offline --manifest-path "$here/../rust/Cargo.toml" --target-dir "$here/../rust/target"
cp "$here/../rust/target/release/libnettls.so" "$MOTE_NATIVE_OUT/"
# The archive keeps only its tls_ symbols global, so its Rust std cannot clash with the one in the runtime.
ld -r --whole-archive "$here/../rust/target/release/libnettls.a" -o "$cache/nettls.o"
objcopy -w --keep-global-symbol='tls_*' --keep-global-symbol='DW.ref.*' "$cache/nettls.o"
rm -f "$MOTE_NATIVE_OUT/libnettls.a"
ar rcs "$MOTE_NATIVE_OUT/libnettls.a" "$cache/nettls.o"
