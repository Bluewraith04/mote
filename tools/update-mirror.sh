#!/usr/bin/env bash
# Copies the committed state of this repository into a checkout of the public mirror, tests it, commits and tags it, and builds the source asset; it pushes nothing.
# Usage: update-mirror.sh VERSION [DEST] [--skip-tests] [--message-file FILE]
set -euo pipefail

root=$(git -C "$(dirname "$0")/.." rev-parse --show-toplevel)
. "$root/tools/mirror-lib.sh"

version=""
dest="$HOME/projects/mote-mirror"
skip_tests=0
message_file=""
positional=0
expect_message=0
for arg in "$@"; do
    if [ "$expect_message" = 1 ]; then
        message_file=$arg
        expect_message=0
        continue
    fi
    case $arg in
        --skip-tests) skip_tests=1 ;;
        --message-file) expect_message=1 ;;
        *)
            positional=$((positional + 1))
            if [ "$positional" = 1 ]; then version=$arg; else dest=$arg; fi
            ;;
    esac
done
[ -n "$version" ] || { echo "usage: update-mirror.sh VERSION [DEST] [--skip-tests] [--message-file FILE]" >&2; exit 2; }
[ -z "$message_file" ] || [ -f "$message_file" ] || { echo "error: no message file $message_file" >&2; exit 2; }
tag="v$version"

mirror_require_clean "$root"
manifest=$(sed -n '/^\[workspace.package\]/,/^\[/ s/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)
if [ "$manifest" != "$version" ]; then
    echo "error: Cargo.toml says $manifest, not $version; bump the manifest first" >&2
    exit 1
fi
[ -d "$dest/.git" ] || { echo "error: $dest is not a checkout of the mirror; clone it first" >&2; exit 1; }
mirror_require_clean "$dest"
if git -C "$dest" rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    echo "error: $dest already has the tag $tag" >&2
    exit 1
fi

pinned=$(git -C "$dest" show HEAD:editors/zed/extension.toml 2>/dev/null | sed -n 's/^rev = "\(.*\)"/\1/p' || true)

trap 'rm -rf "$dest/target"' EXIT
(cd "$dest" && git ls-files -z | xargs -0 rm -f)
find "$dest" -type d -empty -not -path "$dest/.git" -not -path "$dest/.git/*" -delete
mirror_copy "$root" "$dest"
# The old pin stays until the grammar's own commit exists, so a failed run leaves nothing half pinned.
if [ -n "$pinned" ]; then
    sed -i "s/^rev = \".*\"/rev = \"$pinned\"/" "$dest/editors/zed/extension.toml"
fi

restore() { git -C "$dest" checkout -q -- . && git -C "$dest" clean -fdq; }
if ! mirror_check_leaks "$dest"; then
    restore
    exit 1
fi

if [ "$skip_tests" = 0 ]; then
    if ! (cd "$dest" && cargo test --workspace --locked --no-fail-fast); then
        echo "error: the mirror's tests failed; nothing was committed" >&2
        restore
        exit 1
    fi
fi

git -C "$dest" add -A
if git -C "$dest" diff --cached --quiet; then
    echo "no changes since the last mirror commit"
else
    if [ -n "$message_file" ]; then
        { printf 'Mote %s\n\n' "$version"; cat "$message_file"; } | git -C "$dest" commit -q -F -
    else
        git -C "$dest" commit -q -m "Mote $version"
    fi
    # Zed fetches the grammar at `rev`, so it names the last commit that changed the grammar.
    if [ -z "$pinned" ] || ! git -C "$dest" diff --quiet HEAD~1 HEAD -- editors/tree-sitter-mote; then
        sed -i "s/^rev = \".*\"/rev = \"$(git -C "$dest" rev-parse HEAD)\"/" "$dest/editors/zed/extension.toml"
        git -C "$dest" commit -q -am "Pin the Zed grammar to the grammar's last change"
    fi
    echo "committed $version in $dest"
fi

git -C "$dest" tag -a "$tag" -m "Mote $version"
echo "tagged $tag at $(git -C "$dest" rev-parse --short HEAD); push with: git -C $dest push origin main $tag"

"$root/tools/package-source.sh" "$version" --out "$root/target/release-asset"
echo "attach the two files above to the $tag release"
