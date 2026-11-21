#!/usr/bin/env bash
# Parses every `.mote` file and every `mote` block of the markdown files with the tree-sitter grammar and lists those with an ERROR or MISSING node.
# Usage: check-grammar.sh
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
grammar="$root/editors/tree-sitter-mote"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if ! command -v tree-sitter >/dev/null; then
  echo "tree-sitter is not installed" >&2
  exit 2
fi

cd "$root"
find . \( -name target -o -name node_modules -o -name .git \) -prune -o -name '*.mote' -print | sort > "$work/paths"

# Each `mote` block of a markdown file becomes `$work/blocks/N.mote`; `$work/where` names its place.
mkdir "$work/blocks"
count=0
skipped=0
while IFS= read -r doc; do
  case "$doc" in ./tools/fixtures/*|./demo/*) continue ;; esac
  starts=$(grep -n '^```mote$' "$doc" | cut -d: -f1 || true)
  for start in $starts; do
    count=$((count + 1))
    awk -v from="$start" 'NR > from { if ($0 ~ /^```/) exit; print }' "$doc" > "$work/blocks/$count.mote"
    # A block with an `// error` line shows a mistake on purpose.
    if grep -q '// error' "$work/blocks/$count.mote"; then
      skipped=$((skipped + 1))
      continue
    fi
    echo "$work/blocks/$count.mote $doc:$start" >> "$work/where"
    echo "$work/blocks/$count.mote" >> "$work/paths"
  done
done < <(find . \( -name target -o -name node_modules -o -name .git \) -prune -o -name '*.md' -print | sort)

tree-sitter parse -p "$grammar" -j --paths "$work/paths" > "$work/summary" 2>/dev/null || true
total=$(grep -c '"successful"' "$work/summary" || true)
bad=$(grep -B1 '"successful": false' "$work/summary" | grep '"file"' | sed 's/.*"file": "\(.*\)",*/\1/' || true)

failed=0
for file in $bad; do
  failed=$((failed + 1))
  place=$(grep "^$file " "$work/where" 2>/dev/null | cut -d' ' -f2 || true)
  echo "ERROR in ${place:-$file}"
done
echo "$total sources parsed, $failed with an error ($count markdown blocks, $skipped of them marked \`// error\` and left out)"
[ "$failed" -eq 0 ]
