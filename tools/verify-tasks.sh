#!/usr/bin/env bash
# Checks tasks.md against rules 1 to 3 and 11; with --package N, also that every slice of package N is checked.
# Usage: verify-tasks.sh [--package N] [--file F] | --self-test
set -u
cd "$(dirname "$0")/.."

mode=full
file=tasks.md
package=""
while [ $# -gt 0 ]; do
  case "$1" in
    --self-test) mode=selftest ;;
    --package) package="${2:-}"; shift ;;
    --file) file="${2:-}"; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

self_test() {
  local dir=tools/fixtures/tasks passed=0 failed=0 f name want args out status
  for f in "$dir"/pass-*.md "$dir"/fail-*.md; do
    [ -e "$f" ] || continue
    name=$(basename "$f")
    args=$(sed -n 's/^<!-- args: \(.*\) -->$/\1/p' "$f" | head -n 1)
    out=$("$0" --file "$f" $args 2>&1)
    status=$?
    if [[ $name == pass-* ]]; then
      if [ $status -eq 0 ]; then passed=$((passed + 1)); else failed=$((failed + 1)); echo "$name: expected a pass, got:" >&2; echo "$out" >&2; fi
    else
      want=$(sed -n 's/^<!-- expect: \(.*\) -->$/\1/p' "$f" | head -n 1)
      if [ -z "$want" ]; then
        failed=$((failed + 1)); echo "$name: no expect line" >&2
      elif [ $status -eq 0 ]; then
        failed=$((failed + 1)); echo "$name: expected a failure, got a pass" >&2
      elif [[ $out != *"$want"* ]]; then
        failed=$((failed + 1)); echo "$name: failure did not say '$want', got:" >&2; echo "$out" >&2
      else
        passed=$((passed + 1))
      fi
    fi
  done
  echo "self-test: $passed fixtures behaved, $failed did not"
  [ $failed -eq 0 ] && [ $passed -gt 0 ]
}

if [ "$mode" = selftest ]; then
  self_test
  exit $?
fi

if [ ! -f "$file" ]; then
  echo "$file: no such file" >&2
  exit 2
fi

errors=0
fail() { echo "$file:$1: $2" >&2; errors=$((errors + 1)); }

declare -A state pkg_of line_of open_line closed
closes=()
inserted=()
pending_limit=0
last=0
pkg=-1
fence=0
pending=""
lineno=0

re_slice='^- \[( |x)\] \*\*S([0-9]+)(\.[0-9]+)?\*\* '
re_detail='^  needs: (.+) [|] proof: `([^`]+)`( [|] ask: .+)?$'
re_package='^## Work package ([0-9]+(\.[0-9]+)?):'
re_open='^(- )?open (O[0-9]+): .+'

# check_needs LABEL LIMIT LINE TEXT: every slice named must be lower than LIMIT.
check_needs() {
  local n=$1 limit=$2 at=$3 text=$4 token a b
  text=${text// /}
  if [ "$text" = none ]; then return; fi
  IFS=',' read -ra tokens <<< "$text"
  for token in "${tokens[@]}"; do
    if [[ $token =~ ^S([0-9]+)(\.[0-9]+)?$ ]]; then
      a=$((10#${BASH_REMATCH[1]})); b=$a
      if [ -n "${BASH_REMATCH[2]}" ] && [ -z "${state[$a${BASH_REMATCH[2]}]:-}" ]; then
        fail "$at" "S$n: needs $token names an inserted slice that is not declared above it"; continue
      fi
    elif [[ $token =~ ^S([0-9]+)-S([0-9]+)$ ]]; then
      a=$((10#${BASH_REMATCH[1]})); b=$((10#${BASH_REMATCH[2]}))
      if [ "$a" -gt "$b" ]; then fail "$at" "S$n: range $token runs backward"; continue; fi
    else
      fail "$at" "S$n: needs entry '$token' is not none, S<n> or S<a>-S<b>"; continue
    fi
    if [ "$a" -lt 1 ] || [ "$b" -ge "$limit" ]; then
      fail "$at" "S$n: needs $token names a slice that is not lower-numbered"
    fi
  done
}

while IFS= read -r line || [ -n "$line" ]; do
  lineno=$((lineno + 1))
  if [[ $line == '```'* ]]; then fence=$((1 - fence)); continue; fi
  if [ $fence -eq 1 ]; then continue; fi
  if [[ $line =~ $re_package ]]; then
    if [ -n "$pending" ]; then fail "$lineno" "S$pending has no needs and proof line"; pending=""; fi
    pkg=${BASH_REMATCH[1]}
    continue
  fi
  if [[ $line =~ $re_slice ]]; then
    if [ -n "$pending" ]; then fail "$lineno" "S$pending has no needs and proof line"; fi
    mark=${BASH_REMATCH[1]}
    n=$((10#${BASH_REMATCH[2]}))
    key=$n
    limit=$n
    if [ -n "${BASH_REMATCH[3]}" ]; then
      # An inserted slice, such as S107.5, follows S107 and does not move the numbering.
      key=$n${BASH_REMATCH[3]}
      limit=$((n + 1))
      if [ "$n" -ne "$last" ]; then fail "$lineno" "S$key must follow S$n"; fi
      inserted+=("$key")
    else
      if [ "$n" -ne $((last + 1)) ]; then fail "$lineno" "S$n follows S$last; numbers rise by one"; fi
      last=$n
    fi
    pending=$key
    pending_limit=$limit
    state[$key]=$mark
    pkg_of[$key]=$pkg
    line_of[$key]=$lineno
    rest=$line
    re_closes='closes (O[0-9]+)'
    while [[ $rest =~ $re_closes ]]; do
      closes+=("${BASH_REMATCH[1]} $key $lineno")
      rest=${rest#*"${BASH_REMATCH[0]}"}
    done
    continue
  fi
  if [[ $line =~ $re_open ]]; then
    id=${BASH_REMATCH[2]}
    if [ -n "${open_line[$id]:-}" ]; then fail "$lineno" "open item $id is declared twice"; fi
    open_line[$id]=$lineno
    continue
  fi
  if [[ $line == '  needs:'* ]]; then
    if [ -z "$pending" ]; then fail "$lineno" "a needs line follows no slice"; continue; fi
    if [[ $line =~ $re_detail ]]; then
      needs_text=${BASH_REMATCH[1]}
      check_needs "$pending" "$pending_limit" "$lineno" "$needs_text"
    else
      fail "$lineno" "S$pending: needs line is not 'needs: ... | proof: \`command\` [| ask: text]'"
    fi
    pending=""
    continue
  fi
done < "$file"
if [ -n "$pending" ]; then fail "$lineno" "S$pending has no needs and proof line"; fi

for entry in "${closes[@]}"; do
  read -r id n at <<< "$entry"
  if [ -z "${open_line[$id]:-}" ]; then
    fail "$at" "S$n closes $id, which is not an open item"
  elif [ "${open_line[$id]}" -gt "$at" ]; then
    fail "$at" "S$n closes $id, which is declared after it"
  else
    closed[$id]=1
  fi
done
for id in "${!open_line[@]}"; do
  if [ -z "${closed[$id]:-}" ]; then fail "${open_line[$id]}" "open item $id is closed by no later slice"; fi
done

if [ -n "$package" ]; then
  seen=0
  for ((n = 1; n <= last; n++)); do
    if [ "${pkg_of[$n]}" = "$package" ]; then
      seen=$((seen + 1))
      if [ "${state[$n]}" != x ]; then fail "${line_of[$n]}" "S$n of package $package is not checked"; fi
    fi
  done
  for key in "${inserted[@]}"; do
    if [ "${pkg_of[$key]}" = "$package" ]; then
      seen=$((seen + 1))
      if [ "${state[$key]}" != x ]; then fail "${line_of[$key]}" "S$key of package $package is not checked"; fi
    fi
  done
  if [ $seen -eq 0 ]; then fail 1 "package $package has no slices"; fi
fi

if [ $errors -ne 0 ]; then
  echo "$file: $errors problem(s)" >&2
  exit 1
fi
echo "$file: $last slices in order"
