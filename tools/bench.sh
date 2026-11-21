#!/usr/bin/env bash
# Builds and runs the programs under bench/.
#   --run      records bench/results.txt (the median of five runs for each time, the memory of parked tasks)
#   --check    builds each program once, compares its output with the recorded one, and checks the numbers documents cite
#   --compare  runs again and prints each time next to the recorded one
set -eu
cd "$(dirname "$0")/.."

mode=check
case "${1:---check}" in
  --run) mode=run ;;
  --check) mode=check ;;
  --compare) mode=compare ;;
  *) echo "usage: tools/bench.sh [--run | --check | --compare]" >&2; exit 2 ;;
esac

results=bench/results.txt
runs=5
tasks=10000
timed="fib sieve mandelbrot trees maps strings sort closures channel spawn"
memory="idle parked"

cargo build --release --locked -p mote
mote=target/release/mote

# The recorded value of `name key`, or nothing.
recorded() { awk -v n="$1" -v k="$2" '$1 == n && $2 == k { sub(/^[^ ]+ [^ ]+ /, ""); print }' "$results"; }

# Builds bench/$1 and prints the path of its executable.
build() {
  "$mote" build "bench/$1" > /dev/null
  echo "bench/$1/target/$1"
}

# Hundredths of a second since boot; unlike the wall clock it is never stepped (WSL steps the wall clock by about a second now and then).
uptime_centis() { awk '{ printf "%d", $1 * 100 }' /proc/uptime; }

# Milliseconds the executable takes, to ten.
millis() {
  local start end
  start=$(uptime_centis)
  "$1" > /dev/null
  end=$(uptime_centis)
  echo $(((end - start) * 10))
}

# The median of `runs` runs of the executable.
median() {
  for _ in $(seq "$runs"); do
    millis "$1"
  done | sort -n | sed -n "$(((runs + 1) / 2))p"
}

# The resident kilobytes of the executable after it has run for $2 seconds.
resident() {
  "$1" > /dev/null &
  local pid=$!
  sleep "$2"
  awk '/^VmRSS:/ { print $2 }' "/proc/$pid/status"
  kill "$pid"
  wait "$pid" 2> /dev/null || true
}

failures=0
fail() { echo "bench: $1" >&2; failures=$((failures + 1)); }

if [ "$mode" = check ]; then
  [ -f "$results" ] || { fail "$results is missing"; exit 1; }
  for name in $timed; do
    exe=$(build "$name")
    want=$(recorded "$name" output)
    got=$("$exe" | tr '\n' ',' | sed 's/,$//')
    [ "$got" = "$want" ] || fail "$name printed '$got', the results file has '$want'"
    [ -n "$(recorded "$name" ms)" ] || fail "$name has no time in $results"
  done
  for name in $memory; do
    build "$name" > /dev/null
  done
  for key in idle_kb parked_kb cold_kb parked_task_bytes cold_task_bytes; do
    [ -n "$(recorded memory "$key")" ] || fail "memory $key is not in $results"
  done
  for dir in bench/*/; do
    name=$(basename "$dir")
    case " $timed $memory " in *" $name "*) ;; *) fail "bench/$name is not run by tools/bench.sh" ;; esac
  done
  # A document cites a number with <!-- bench: name key --> after it; the number must be the recorded one.
  while IFS= read -r line; do
    path=${line%%:*}
    text=${line#*:}
    text=${text#*:}
    while read -r name key; do
      value=$(recorded "$name" "$key")
      [ -n "$value" ] || { fail "$path cites $name $key, which the results file lacks"; continue; }
      case "$text" in *"$value"*) ;; *) fail "$path cites $name $key but not as $value" ;; esac
    done < <(echo "$text" | grep -o '<!-- bench: [a-z_]* [a-z_]* -->' | sed 's/<!-- bench: \(.*\) -->/\1/')
  done < <(grep -rn "<!-- bench:" docs tasks.md || true)
  [ "$failures" -eq 0 ]
  exit $?
fi

if [ "$mode" = run ]; then
  {
    echo "# machine $(lscpu | sed -n 's/^Model name: *//p'), $(nproc) threads"
    echo "# mote $(git rev-parse --short HEAD), $(date +%Y-%m-%d)"
    echo "# name key value: ms is the median of $runs runs; memory is the resident kilobytes (parked_kb 0.9 s into a run of 10000 parked tasks, cold_kb 4 s in, idle_kb of a run with none)"
  } > "$results.new"
else
  : > "$results.new"
fi

for name in $timed; do
  exe=$(build "$name")
  took=$(median "$exe")
  echo "$name output $("$exe" | tr '\n' ',' | sed 's/,$//')" >> "$results.new"
  echo "$name ms $took" >> "$results.new"
  if [ "$mode" = compare ]; then
    echo "$name: $took ms, recorded $(recorded "$name" ms) ms"
  fi
done

idle=$(build idle)
parked=$(build parked)
idle_kb=$(resident "$idle" 0.9)
parked_kb=$(resident "$parked" 0.9)
cold_kb=$(resident "$parked" 4)
{
  echo "memory idle_kb $idle_kb"
  echo "memory parked_kb $parked_kb"
  echo "memory cold_kb $cold_kb"
  echo "memory parked_task_bytes $(((parked_kb - idle_kb) * 1024 / tasks))"
  echo "memory cold_task_bytes $(((cold_kb - idle_kb) * 1024 / tasks))"
} >> "$results.new"

if [ "$mode" = run ]; then
  mv "$results.new" "$results"
  echo "wrote $results"
else
  cat "$results.new"
  rm "$results.new"
fi
