#!/usr/bin/env bash
# Run the whole workspace test suite with every `VM::run` on 1, 4 and all-core
# workers. A failure at any count is a parallel-safety bug.
set -euo pipefail
cd "$(dirname "$0")/.."
cores=$(nproc 2>/dev/null || echo 4)
for w in $(printf '%s\n' 1 4 "$cores" | sort -nu); do
    echo "=== MOTE_TEST_WORKERS=$w"
    MOTE_TEST_WORKERS=$w cargo test --workspace
done
