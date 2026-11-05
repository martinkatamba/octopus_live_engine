#!/bin/sh
# Builds the extension and puts it next to python/octopus_core, so
# PYTHONPATH=python imports it. `maturin build` makes wheels (phase 3).
#   ./build.sh [--release]    PYTHON=/path/to/python to pick the interpreter,
#                             FEATURES=duckdb to add optional stores
set -eu
cd "$(dirname "$0")"
PY="${PYTHON:-../../../python/.venv/bin/python}"
# Absolute: pyo3 runs the interpreter from its own build directory.
PY="$(cd "$(dirname "$PY")" && pwd)/$(basename "$PY")"
# The Temporal SDK's build needs protoc; use the vendored one when none is set.
if [ -z "${PROTOC:-}" ] && ! command -v protoc >/dev/null 2>&1; then
  case "$(uname -s)-$(uname -m)" in
    Darwin-x86_64) PLAT=macos-x86_64 ;;
    Darwin-arm64) PLAT=macos-aarch_64 ;;
    Linux-x86_64) PLAT=linux-x86_64 ;;
    Linux-aarch64) PLAT=linux-aarch_64 ;;
    *) PLAT=none ;;
  esac
  for p in "$HOME"/.cargo/registry/src/*/protoc-bin-vendored-"$PLAT"-*/bin/protoc; do
    [ -x "$p" ] && export PROTOC="$p" && break
  done
fi
PROFILE=debug
[ "${1:-}" = "--release" ] && PROFILE=release
PYO3_PYTHON="$PY" cargo build $([ "$PROFILE" = release ] && echo --release) ${FEATURES:+--features "$FEATURES"}
SUFFIX="$("$PY" -c 'import sysconfig; print(sysconfig.get_config_var("EXT_SUFFIX"))')"
LIB=../../target/$PROFILE/lib_native.dylib
[ -f "$LIB" ] || LIB=../../target/$PROFILE/lib_native.so
# Copy, then rename: a process that has the old library loaded keeps it.
cp "$LIB" "python/octopus_core/.native$SUFFIX.tmp"
mv -f "python/octopus_core/.native$SUFFIX.tmp" "python/octopus_core/_native$SUFFIX"
echo "built python/octopus_core/_native$SUFFIX ($PROFILE)"
