# Shared by update-mirror.sh: what the public mirror holds and the checks on it.

# Copies the tracked files of the repository at $1 into $2, without the offline setting of the development machine.
mirror_copy() {
    local root=$1 dest=$2
    (
        cd "$root"
        git ls-files -z -- . ':!:.cargo/config.toml' | tar --null -T - -cf - | tar -xf - -C "$dest"
    )
    # `tools/check.sh` asks for that setting, which the mirror does not have.
    sed -i "/offline = true/d" "$dest/tools/check.sh"
    if grep -q "offline = true" "$dest/tools/check.sh"; then
        echo "error: tools/check.sh still asks for offline = true" >&2
        return 1
    fi
}

# Fails when a file of $1 names a private path or note.
mirror_check_leaks() {
    local leaks
    leaks=$(grep -rIn -E 'ADR[ -]?[0-9]{4}|docs/adr|/home/|Checkpoint\.md' "$1" --exclude-dir=.git --exclude-dir=target || true)
    if [ -n "$leaks" ]; then
        echo "error: private references in the mirror:" >&2
        echo "$leaks" >&2
        return 1
    fi
}

mirror_require_clean() {
    if [ -n "$(git -C "$1" status --porcelain --untracked-files=no)" ]; then
        echo "error: commit the working tree of $1 first; the mirror is built from committed files" >&2
        return 1
    fi
}
