#!/bin/sh
# Checks the C ABI: include/octopus.h matches the exports, and a C program
# built against the header and the library runs. Run from anywhere; CI runs it.
set -eu
CORE="$(cd "$(dirname "$0")/.." && pwd)"
FFI="$CORE/crates/octopus-ffi"
cd "$FFI"
cbindgen --config cbindgen.toml --output include/octopus.h --verify
cargo build -p octopus-live-ffi --no-default-features --quiet
OUT="$CORE/target/debug"
cc -std=c99 -D_GNU_SOURCE -Wall -Werror -I include tests/c/smoke.c -L "$OUT" -loctopus_ffi -o "$OUT/octopus-ffi-smoke"
DYLD_LIBRARY_PATH="$OUT" LD_LIBRARY_PATH="$OUT" "$OUT/octopus-ffi-smoke"
