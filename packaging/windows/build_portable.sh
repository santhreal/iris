#!/usr/bin/env bash
set -euo pipefail

# build_portable.sh — Builds the Windows portable zip for iris
#
# References:
#   packaging/CONTRACT.md
#
# Release asset name:
#   iris-{ver}-windows-x86_64-portable.zip
#   iris-{ver}-windows-x86_64-portable.zip.sha256
#
# Contents of the zip (the updater reads iris\iris.exe,
# src/bin/iris/sys/install/windows.rs):
#   iris/iris.exe
#   iris/LICENSE-APACHE
#   iris/LICENSE-MIT
#
# Usage:
#   ./build_portable.sh [options]
#
# Options:
#   -b, --binary <path>    Path to iris.exe (default: target/release/iris.exe)
#   -o, --out <dir>        Output directory (default: dist)
#   -v, --version <ver>    App version (default: from Cargo.toml)
#   -h, --help             Show this help message

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

IRIS_BIN="$REPO_ROOT/target/release/iris.exe"
OUT_DIR="$REPO_ROOT/dist"
VERSION=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    -b|--binary)
      IRIS_BIN="$2"
      shift 2
      ;;
    -o|--out)
      OUT_DIR="$2"
      shift 2
      ;;
    -v|--version)
      VERSION="$2"
      shift 2
      ;;
    -h|--help)
      awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; seen = 1; next } seen { exit }' "$0"
      exit 0
      ;;
    *)
      echo "Error: unknown argument $1 (see --help)" >&2
      exit 1
      ;;
  esac
done

if [[ -z "$VERSION" ]]; then
  VERSION=$(sed -nE 's/^version = "([^"]+)"/\1/p' "$REPO_ROOT/Cargo.toml" | head -n1)
fi
if [[ ! -f "$IRIS_BIN" ]]; then
  echo "Error: $IRIS_BIN not found; build it with: cargo build --release --bin iris" >&2
  exit 1
fi

ZIP_NAME="iris-${VERSION}-windows-x86_64-portable.zip"
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
rm -f "$OUT_DIR/$ZIP_NAME" "$OUT_DIR/$ZIP_NAME.sha256"

STAGING_DIR="$(mktemp -d "$OUT_DIR/.portable.XXXXXX")"
trap 'rm -rf "$STAGING_DIR"' EXIT
mkdir "$STAGING_DIR/iris"
cp "$IRIS_BIN" "$STAGING_DIR/iris/iris.exe"
cp "$REPO_ROOT/LICENSE-APACHE" "$REPO_ROOT/LICENSE-MIT" "$STAGING_DIR/iris/"

echo "==> Building $OUT_DIR/$ZIP_NAME"
# The updater unpacks the zip with the tar.exe Windows ships (bsdtar),
# which writes it here on Windows. zip or Python's zipfile writes one it
# reads elsewhere.
WIN_TAR=""
SYSROOT="${SYSTEMROOT:-${SystemRoot:-}}"
if [[ -n "$SYSROOT" ]]; then
  command -v cygpath >/dev/null 2>&1 && SYSROOT="$(cygpath -u "$SYSROOT")"
  WIN_TAR="$SYSROOT/System32/tar.exe"
fi
if [[ -n "$WIN_TAR" && -f "$WIN_TAR" ]]; then
  (cd "$STAGING_DIR" && "$WIN_TAR" -a -cf iris.zip iris)
elif command -v zip >/dev/null 2>&1; then
  (cd "$STAGING_DIR" && zip -qrX iris.zip iris)
elif command -v python3 >/dev/null 2>&1; then
  (cd "$STAGING_DIR" && python3 -m zipfile -c iris.zip iris)
else
  echo "Error: building the zip needs zip or python3 on PATH" >&2
  exit 1
fi
mv "$STAGING_DIR/iris.zip" "$OUT_DIR/$ZIP_NAME"

if command -v sha256sum >/dev/null 2>&1; then
  (cd "$OUT_DIR" && sha256sum "$ZIP_NAME" > "$ZIP_NAME.sha256")
else
  (cd "$OUT_DIR" && shasum -a 256 "$ZIP_NAME" > "$ZIP_NAME.sha256")
fi

echo "==> Created $OUT_DIR/$ZIP_NAME"
