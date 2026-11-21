#!/usr/bin/env bash
# Runs every check of the repository: pins, tasks.md, clippy with warnings denied, and the tests.
set -eu
cd "$(dirname "$0")/.."

want=$(tr -d '[:space:]' < tools/rust-version)
have=$(rustc --version | awk '{print $2}')
if [ "$have" != "$want" ]; then
  echo "rustc is $have; tools/rust-version records $want" >&2
  exit 1
fi

git ls-files --error-unmatch Cargo.lock > /dev/null
grep -qx 'channel = "stable"' rust-toolchain.toml
grep -qx 'components = \["clippy"\]' rust-toolchain.toml

tools/verify-tasks.sh
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked --no-fail-fast
