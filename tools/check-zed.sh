#!/usr/bin/env bash
# Checks the Zed extension: its files exist, its manifest names a grammar, and each query compiles against the grammar and matches a real file.
# Usage: check-zed.sh
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
zed="$root/editors/zed"
grammar="$root/editors/tree-sitter-mote"
language="$zed/languages/mote"
sample="$root/mote_std/pane/src/element.mote"
failed=0

fail() {
  echo "FAIL: $1" >&2
  failed=$((failed + 1))
}

if ! command -v tree-sitter >/dev/null; then
  echo "tree-sitter is not installed" >&2
  exit 2
fi

for file in "$zed/extension.toml" "$language/config.toml" "$language/highlights.scm" "$language/brackets.scm" "$language/indents.scm" "$language/outline.scm" "$language/overrides.scm"; do
  [ -f "$file" ] || fail "missing ${file#"$root"/}"
done

for key in id name version schema_version '\[grammars.mote\]' repository rev path; do
  grep -q "^$key" "$zed/extension.toml" || fail "extension.toml has no $key"
done
grep -q '^grammar = "mote"' "$language/config.toml" || fail "config.toml does not name the grammar mote"

# A query that does not compile makes `tree-sitter query` exit nonzero; each must also match something in the sample.
for query in highlights brackets indents outline overrides; do
  [ -f "$language/$query.scm" ] || continue
  if ! out=$(tree-sitter query -p "$grammar" "$language/$query.scm" "$sample" 2>&1); then
    fail "$query.scm does not compile: $(echo "$out" | grep -i error | head -1)"
  elif ! echo "$out" | grep -q "capture:"; then
    fail "$query.scm matches nothing in ${sample#"$root"/}"
  fi
done

# The grammar's own highlight path is this file.
grep -q 'languages/mote/highlights.scm' "$grammar/tree-sitter.json" || fail "tree-sitter.json does not point at highlights.scm"

[ "$failed" -eq 0 ] && echo "zed extension ok"
[ "$failed" -eq 0 ]
