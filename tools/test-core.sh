#!/usr/bin/env bash
# Builds the native of a core library when it has one, then runs `mote test` on it; $MOTE names the command.
# Usage: test-core.sh NAME [--core DIR] [-- FILTER]
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
core="$root/core"
name=""
filter=""
while [ $# -gt 0 ]; do
  case "$1" in
    --core) core=$(cd "${2:?--core needs a directory}" && pwd); shift ;;
    --) filter="${2:?-- needs a filter}"; shift ;;
    -*) echo "unknown argument: $1" >&2; exit 2 ;;
    *) name="$1" ;;
  esac
  shift
done
if [ -z "$name" ]; then
  echo "usage: test-core.sh NAME [--core DIR]" >&2
  exit 2
fi
if [ ! -f "$core/$name/mote.toml" ]; then
  echo "no library $name in $core" >&2
  exit 1
fi

# The libraries `name` imports, and the ones they import, each once.
seen=" $name "
pending="$name"
while [ -n "$pending" ]; do
  current=${pending%% *}
  if [ "$current" = "$pending" ]; then pending=""; else pending=${pending#* }; fi
  for used in $(grep -rhoE --include='*.mote' 'from [a-z_][a-z0-9_]*' "$core/$current/src" "$core/$current/tests" 2>/dev/null | sed 's/^from //' | sort -u); do
    case "$seen" in *" $used "*) continue ;; esac
    [ -f "$core/$used/mote.toml" ] || continue
    seen="$seen$used "
    pending="${pending:+$pending }$used"
  done
done

for library in $seen; do
  "$root/tools/build-core-natives.sh" --core "$core" "$library"
done
if [ -n "$filter" ]; then
  "${MOTE:-mote}" test --core "$core" --filter "$filter" "$core/$name"
else
  "${MOTE:-mote}" test --core "$core" "$core/$name"
fi
