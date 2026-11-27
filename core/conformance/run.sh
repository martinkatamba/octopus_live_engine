#!/usr/bin/env bash
# Runs every conformance case on the Go, Rust, Python and .NET live engines,
# on the .NET SDK over the core, and on octopus-core directly, and writes
# results/report.md. Pass --strict to exit non-zero on any failure. The .NET
# engine is reported but not gated: the core is replacing it.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
results="$here/results"
mkdir -p "$results"
python="${PYTHON:-$(command -v "$here/../../../.venv/bin/python" || command -v python3)}"

(cd "$here/runners/go" && go run . "$here/cases") > "$results/go.json" 2> "$results/go.stderr"
(cd "$here/runners/rust" && cargo run --quiet --release -- "$here/cases") > "$results/rust.json" 2> "$results/rust.stderr"
(cd "$here/runners/core" && cargo run --quiet --release -- "$here/cases") > "$results/core.json" 2> "$results/core.stderr"
"$python" "$here/runners/python/run.py" "$here/cases" > "$results/python.json" 2> "$results/python.stderr"

# The .NET SDK loads the core's C ABI from core/target.
(cd "$here/.." && cargo build --quiet -p octopus-live-ffi) 2> "$results/dotnet-build.stderr"
(cd "$here/runners/dotnet" && dotnet build -c Release -nologo -v quiet) > "$results/dotnet-build.log" 2>> "$results/dotnet-build.stderr"
for engine in dotnet core; do
  name="dotnet"; [ "$engine" = core ] && name="dotnet-core"
  (cd "$here/runners/dotnet" && dotnet run -c Release --no-build -- "$here/cases" "$engine") > "$results/$name.json" 2> "$results/$name.stderr"
done

python3 "$here/check.py" "$results/go.json" "$results/rust.json" "$results/python.json" "$results/dotnet.json" "$results/dotnet-core.json" "$results/core.json" \
  --report "$results/report.md" --report-only dotnet "$@"
