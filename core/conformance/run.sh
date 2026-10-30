#!/usr/bin/env bash
# Runs every conformance case on the Go, Rust and Python live engines, and on
# octopus-core directly, and writes
# results/report.md. Pass --strict to exit non-zero on any failure.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
results="$here/results"
mkdir -p "$results"
python="${PYTHON:-$(command -v "$here/../../../.venv/bin/python" || command -v python3)}"

(cd "$here/runners/go" && go run . "$here/cases") > "$results/go.json" 2> "$results/go.stderr"
(cd "$here/runners/rust" && cargo run --quiet --release -- "$here/cases") > "$results/rust.json" 2> "$results/rust.stderr"
(cd "$here/runners/core" && cargo run --quiet --release -- "$here/cases") > "$results/core.json" 2> "$results/core.stderr"
"$python" "$here/runners/python/run.py" "$here/cases" > "$results/python.json" 2> "$results/python.stderr"

python3 "$here/check.py" "$results/go.json" "$results/rust.json" "$results/python.json" "$results/core.json" --report "$results/report.md" "$@"
