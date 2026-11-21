#!/usr/bin/env bash
# Builds mote-source-VERSION.tar.gz (std/, core/ and the natives of each triple) and its .sha256; the same tree and natives give the same bytes.
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

declares_native() { grep -rqs --include='*.mote' '@link(' "$1/src"; }

# Builds the natives in place (a fresh build is a no-op for cargo) so `native/$triple` of each library is current.
"$root/tools/build-core-natives.sh" >&2

# Writes $1/$name.tar.gz and $1/$name.tar.gz.sha256 from the tree.
package() {
  local dest=$1 stage="$work/stage"
  rm -rf "$stage"
  mkdir -p "$stage/$name"
  # Sources only: no Rust `target` folders and no built natives from the tree; the natives come back below.
  tar -C "$root" --exclude=rust/target --exclude="native/$triple" -cf - std core | tar -C "$stage/$name" -xf -
  local dir lib
  for dir in "$root"/core/*/; do
    dir=${dir%/}
    [ -d "$dir/native/$triple" ] || continue
    mkdir -p "$stage/$name/core/$(basename "$dir")/native/$triple"
    for lib in "$dir/native/$triple"/*; do
      [ -f "$lib" ] && cp "$lib" "$stage/$name/core/$(basename "$dir")/native/$triple/"
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

has std/mote.toml || fail "no std/mote.toml"
libraries=0
natives=0
for dir in "$root"/core/*/; do
  dir=${dir%/}
  [ -f "$dir/mote.toml" ] || continue
  lib=$(basename "$dir")
  libraries=$((libraries + 1))
  has "core/$lib/mote.toml" || fail "no core/$lib/mote.toml"
  if declares_native "$dir"; then
    natives=$((natives + 1))
    has "core/$lib/native/$triple/lib$lib.so" || fail "no native core/$lib/native/$triple/lib$lib.so"
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
  echo "ok: $libraries libraries, $natives with a native, $(wc -l < "$work/list") entries"
fi
[ $failed -eq 0 ]
