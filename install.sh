#!/usr/bin/env bash
# Installs the `mote` CLI via `cargo install` and copies `mote_std/` to `$MOTE_HOME/mote_std` (default `~/.mote/mote_std`).
# Usage: ./install.sh [--compile-std]
set -eu
root=$(cd "$(dirname "$0")" && pwd)
mote_home=${MOTE_HOME:-"$HOME/.mote"}

if [ "${1:-}" = "--compile-std" ]; then
  "$root/compile-std.sh"
fi

cargo install --path "$root/mote" --locked

dest="$mote_home/mote_std"
stage="$mote_home/.mote_std.$$"
mkdir -p "$mote_home"
rm -rf "$stage"
mkdir -p "$stage"
tar -C "$root/mote_std" --exclude='rust/target' --exclude='target' -cf - . | tar -C "$stage" -xf -
rm -rf "$dest"
mv "$stage" "$dest"
echo "installed mote_std -> $dest"
