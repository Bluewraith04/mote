#!/usr/bin/env bash
# Builds mote-source-VERSION.tar.gz (mote_std/ and the compiled libraries of each triple) and its .sha256; the same tree and libraries give the same bytes.
# Usage: package-source.sh VERSION [--out DIR] | package-source.sh --check [--twice]
# --check builds into a scratch folder and tests the archive; --twice builds it a second time and compares the two.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
triple=x86_64-linux
version=""
out="$root/target/source-asset"
check=no
twice=no
while [ $# -gt 0 ]; do
  case "$1" in
    --check) check=yes ;;
    --twice) twice=yes ;;
    --out) mkdir -p "${2:?--out needs a directory}"; out=$(cd "$2" && pwd); shift ;;
    -*) echo "unknown argument: $1" >&2; exit 2 ;;
    *) version="$1" ;;
  esac
  shift
done
if [ $check = yes ]; then
  version=${version:-check}
fi
if [ -z "$version" ]; then
  echo "usage: package-source.sh VERSION [--out DIR] | --check [--twice]" >&2
  exit 2
fi
if [ $twice = yes ] && [ $check = no ]; then
  echo "--twice goes with --check" >&2
  exit 2
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
if [ $check = yes ]; then
  out="$work/out"
fi
mkdir -p "$out"

name="mote-source-$version"

declares_extern() { grep -rqEs --include='*.mote' '^[[:space:]]*(pub[[:space:]]+)?extern[[:space:]]+"' "$1/src"; }

# Builds the external libraries in place (a fresh build is a no-op for cargo) so `lib/$triple` of each library is current.
"$root/tools/build-std-libs.sh" >&2

# Writes $1/$name.tar.gz and $1/$name.tar.gz.sha256 from the tree.
package() {
  local dest=$1 stage="$work/stage"
  rm -rf "$stage"
  mkdir -p "$stage/$name"
  # Sources only: no Rust `target` folders and no built libraries from the tree; they are copied back below.
  tar -C "$root" --exclude=rust/target --exclude="lib/$triple" -cf - mote_std | tar -C "$stage/$name" -xf -
  local dir lib
  for dir in "$root"/mote_std/*/; do
    dir=${dir%/}
    [ -d "$dir/lib/$triple" ] || continue
    mkdir -p "$stage/$name/mote_std/$(basename "$dir")/lib/$triple"
    for lib in "$dir/lib/$triple"/*; do
      [ -f "$lib" ] && cp "$lib" "$stage/$name/mote_std/$(basename "$dir")/lib/$triple/"
    done
  done
  # Fixed order, owner, times and modes; `gzip -n` leaves out the name and time.
  tar -C "$stage" --sort=name --format=gnu --owner=0 --group=0 --numeric-owner --mtime="@${SOURCE_DATE_EPOCH:-0}" --mode='u+rw,go+r,go-w,a+X' -cf - "$name" | gzip -n -9 > "$dest/$name.tar.gz"
  (cd "$dest" && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
}

package "$out"

if [ $check = no ]; then
  echo "$out/$name.tar.gz"
  cat "$out/$name.tar.gz.sha256"
  exit 0
fi

failed=0
fail() {
  echo "FAIL: $1" >&2
  failed=$((failed + 1))
}

(cd "$out" && sha256sum -c --quiet "$name.tar.gz.sha256") || fail "the .sha256 does not match the archive"
tar -tzf "$out/$name.tar.gz" > "$work/list"
has() { grep -qxF "$name/$1" "$work/list"; }

has mote_std/std/mote.toml || fail "no mote_std/std/mote.toml"
libraries=0
built_libs=0
for dir in "$root"/mote_std/*/; do
  dir=${dir%/}
  [ -f "$dir/mote.toml" ] || continue
  lib=$(basename "$dir")
  [ "$lib" = "std" ] && continue
  libraries=$((libraries + 1))
  has "mote_std/$lib/mote.toml" || fail "no mote_std/$lib/mote.toml"
  if declares_extern "$dir"; then
    built_libs=$((built_libs + 1))
    has "mote_std/$lib/lib/$triple/lib$lib.so" || fail "no library mote_std/$lib/lib/$triple/lib$lib.so"
  fi
done
if grep -q '/target/' "$work/list"; then
  fail "the archive holds a build folder: $(grep -m1 '/target/' "$work/list")"
fi
if tar -tvzf "$out/$name.tar.gz" | grep -vq ' root/root \| 0/0 '; then
  fail "a file in the archive has an owner"
fi

if [ $twice = yes ]; then
  mkdir "$work/again"
  package "$work/again"
  if cmp -s "$out/$name.tar.gz" "$work/again/$name.tar.gz"; then
    echo "twice: the two archives are the same ($(cut -d' ' -f1 "$out/$name.tar.gz.sha256"))"
  else
    fail "the second archive differs from the first"
  fi
fi

if [ $failed -eq 0 ]; then
  echo "ok: $libraries libraries, $built_libs with a shared library, $(wc -l < "$work/list") entries"
fi
[ $failed -eq 0 ]
