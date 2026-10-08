#!/usr/bin/env bash
# build_portable.sh: builds the portable zip of iris for Windows.
#
#   packaging/windows/build_portable.sh --bin <path> --out <dir>
#       [--version <ver>] [--arch x86_64|aarch64] [--skip-sha]
#
#   --bin <path>      the iris.exe to package
#   --out <dir>       the directory the zip is written to
#   --version <ver>   the iris version (default: Cargo.toml)
#   --arch <arch>     x86_64 or aarch64 (default: iris.exe's); fails
#                     when iris.exe is built for another
#   --skip-sha        write no .sha256 sidecar
#
# Writes iris-{ver}-windows-{arch}-portable.zip (packaging/CONTRACT.md)
# and its .sha256 sidecar. The zip holds one directory, iris, with
# iris.exe (the updater reads iris\iris.exe,
# src/bin/iris/sys/install/windows.rs), LICENSE-MIT, LICENSE-APACHE,
# and Inter-OFL.txt.
set -euo pipefail
# Zipped directories are 0755 and files 0644 whatever the caller's umask.
umask 022

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
# shellcheck source=packaging/lib.sh
. "$REPO_ROOT/packaging/lib.sh"

IRIS_BIN=""
OUT_DIR=""
VERSION=""
ARCH=""
GEN_SHA=true
while [[ $# -gt 0 ]]; do
  case "$1" in
    --bin) IRIS_BIN=${2:?--bin needs a path}; shift 2 ;;
    --out) OUT_DIR=${2:?--out needs a directory}; shift 2 ;;
    --version) VERSION=${2:?--version needs a version}; shift 2 ;;
    --arch) ARCH=${2:?--arch needs x86_64 or aarch64}; shift 2 ;;
    --skip-sha) GEN_SHA=false; shift ;;
    -h|--help) iris_help "$0"; exit 0 ;;
    *) echo "Error: unknown argument $1 (see --help)" >&2; exit 2 ;;
  esac
done
if [[ -z "$IRIS_BIN" || -z "$OUT_DIR" ]]; then
  echo "Error: --bin and --out are required (see --help)" >&2
  exit 2
fi
if [[ ! -f "$IRIS_BIN" ]]; then
  echo "Error: $IRIS_BIN does not exist" >&2
  exit 1
fi

VERSION=${VERSION:-$(iris_cargo_version "$REPO_ROOT")}
ARCH=$(iris_resolve_arch "$IRIS_BIN" "$ARCH")

mkdir -p "$OUT_DIR"
ZIP_NAME="iris-${VERSION}-windows-${ARCH}-portable.zip"
ZIP_PATH="$(cd "$OUT_DIR" && pwd)/$ZIP_NAME"
echo "Packaging $IRIS_BIN as $ZIP_PATH"
rm -f "$ZIP_PATH" "$ZIP_PATH.sha256"

STAGING_DIR=$(iris_staging_dir "$REPO_ROOT" portable)
trap 'rm -rf "$STAGING_DIR"' EXIT
mkdir "$STAGING_DIR/iris"
cp "$IRIS_BIN" "$STAGING_DIR/iris/iris.exe"
iris_license_files "$REPO_ROOT" "$STAGING_DIR/iris"

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
  echo "Error: building the zip requires zip or python3 on PATH" >&2
  exit 1
fi
mv "$STAGING_DIR/iris.zip" "$ZIP_PATH"
if [[ "$GEN_SHA" == true ]]; then
  iris_sha256_sidecar "$ZIP_PATH"
fi
echo "Built $ZIP_PATH"
