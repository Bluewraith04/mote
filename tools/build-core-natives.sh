#!/usr/bin/env bash
# Builds the native of each core library into native/x86_64-linux/ of its package.
# A library with a native has `native/build.sh` (run in the package with MOTE_NATIVE_OUT and MOTE_HEADER_DIR set) or a `rust/` crate.
# Usage: build-core-natives.sh [--check] [--core DIR] [--out DIR] [name]
# With --out the files of each library go to DIR/<name>/ and its Rust build to DIR/<name>/rust-target, and nothing is written below the core folder.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
core="$root/core"
check=no
only=""
outroot=""
while [ $# -gt 0 ]; do
  case "$1" in
    --check) check=yes ;;
    --core) core=$(cd "${2:?--core needs a directory}" && pwd); shift ;;
    --out) mkdir -p "${2:?--out needs a directory}"; outroot=$(cd "$2" && pwd); shift ;;
    -*) echo "unknown argument: $1" >&2; exit 2 ;;
    *) only="$1" ;;
  esac
  shift
done

triple=x86_64-linux

declares_native() { grep -rqs --include='*.mote' '@link(' "$1/src"; }
has_builder() { [ -x "$1/native/build.sh" ] || [ -f "$1/rust/Cargo.toml" ]; }

build() {
  local dir=$1 name out target
  name=$(basename "$dir")
  out="$dir/native/$triple"
  target="$dir/rust/target"
  if [ -n "$outroot" ]; then
    out="$outroot/$name"
    target="$outroot/$name/rust-target"
  fi
  mkdir -p "$out"
  if [ -x "$dir/native/build.sh" ]; then
    (cd "$dir" && MOTE_NATIVE_OUT="$out" MOTE_HEADER_DIR="$root/abi" ./native/build.sh)
  else
    cargo build --release --offline --manifest-path "$dir/rust/Cargo.toml" --target-dir "$target"
    # Each file is renamed into place, so a build running at the same time never sees half of one.
    cp "$target/release/lib$name.so" "$out/.lib$name.so.$$"
    mv -f "$out/.lib$name.so.$$" "$out/lib$name.so"
    if [ -f "$target/release/lib$name.a" ]; then
      # The archive keeps only its `<name>_` symbols global, so its Rust std cannot clash with the runtime's.
      local work
      work=$(mktemp -d)
      ld -r --whole-archive "$target/release/lib$name.a" -o "$work/$name.o"
      objcopy -w --keep-global-symbol="${name}_*" --keep-global-symbol='DW.ref.*' "$work/$name.o"
      ar rcs "$out/.lib$name.a.$$" "$work/$name.o"
      mv -f "$out/.lib$name.a.$$" "$out/lib$name.a"
      rm -rf "$work"
    fi
  fi
  if [ ! -f "$out/lib$name.so" ]; then
    echo "$name: the build wrote no $out/lib$name.so" >&2
    return 1
  fi
  echo "built $out/lib$name.so"
}

if [ -n "$only" ] && [ ! -f "$core/$only/mote.toml" ]; then
  echo "no library $only in $core" >&2
  exit 1
fi

failed=0
natives=0
libraries=0
for dir in "$core"/*/; do
  dir=${dir%/}
  [ -f "$dir/mote.toml" ] || continue
  name=$(basename "$dir")
  if [ -n "$only" ] && [ "$name" != "$only" ]; then continue; fi
  libraries=$((libraries + 1))
  declares_native "$dir" || continue
  natives=$((natives + 1))
  if ! has_builder "$dir"; then
    echo "$name: declares a native and has no native/build.sh or rust/Cargo.toml" >&2
    failed=$((failed + 1))
  elif [ $check = no ]; then
    build "$dir" || failed=$((failed + 1))
  fi
done

if [ $failed -ne 0 ]; then
  exit 1
fi
if [ $check = yes ]; then
  echo "ok: $libraries libraries, $natives with a native"
fi
