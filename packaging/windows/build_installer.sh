#!/usr/bin/env bash
# build_installer.sh: builds the NSIS setup program of iris for Windows
# with makensis and packaging/windows/iris.nsi.
#
#   packaging/windows/build_installer.sh --bin <path> --out <dir>
#       [--version <ver>] [--arch x86_64|aarch64] [--skip-sha]
#   packaging/windows/build_installer.sh --check [--version <ver>]
#       [--arch x86_64|aarch64]
#
#   --bin <path>      the iris.exe to package
#   --out <dir>       the directory the setup program is written to
#   --version <ver>   the iris version (default: Cargo.toml)
#   --arch <arch>     x86_64 or aarch64 (default: iris.exe's); fails
#                     when iris.exe is built for another
#   --skip-sha        write no .sha256 sidecar
#   --check           compile iris.nsi around an empty iris.exe for
#                     --arch (default x86_64) and delete the result;
#                     writes nothing to --out
#
# Writes iris-{ver}-windows-{arch}-setup.exe (packaging/CONTRACT.md) and
# its .sha256 sidecar. The setup program's numeric version is the
# major.minor.patch of {ver} followed by .0; its ProductVersion string
# is {ver}. Requires makensis (NSIS 3).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
# shellcheck source=packaging/lib.sh
. "$REPO_ROOT/packaging/lib.sh"

IRIS_BIN=""
OUT_DIR=""
VERSION=""
ARCH=""
GEN_SHA=true
CHECK=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --bin) IRIS_BIN=${2:?--bin needs a path}; shift 2 ;;
    --out) OUT_DIR=${2:?--out needs a directory}; shift 2 ;;
    --version) VERSION=${2:?--version needs a version}; shift 2 ;;
    --arch) ARCH=${2:?--arch needs x86_64 or aarch64}; shift 2 ;;
    --skip-sha) GEN_SHA=false; shift ;;
    --check) CHECK=true; shift ;;
    -h|--help) iris_help "$0"; exit 0 ;;
    *) echo "Error: unknown argument $1 (see --help)" >&2; exit 2 ;;
  esac
done

VERSION=${VERSION:-$(iris_cargo_version "$REPO_ROOT")}
VERSION_QUAD="$(iris_core_version "$VERSION").0"

STAGING_DIR=$(iris_staging_dir "$REPO_ROOT" installer)
trap 'rm -rf "$STAGING_DIR"' EXIT

if [[ "$CHECK" == true ]]; then
  case "${ARCH:-x86_64}" in
    x86_64|amd64) ARCH=x86_64 ;;
    aarch64|arm64) ARCH=aarch64 ;;
    *) echo "Error: --arch $ARCH is not x86_64 or aarch64" >&2; exit 2 ;;
  esac
  IRIS_BIN="$STAGING_DIR/iris.exe"
  : >"$IRIS_BIN"
  OUT_DIR="$STAGING_DIR"
else
  if [[ -z "$IRIS_BIN" || -z "$OUT_DIR" ]]; then
    echo "Error: --bin and --out are required (see --help)" >&2
    exit 2
  fi
  if [[ ! -f "$IRIS_BIN" ]]; then
    echo "Error: $IRIS_BIN does not exist" >&2
    exit 1
  fi
  ARCH=$(iris_resolve_arch "$IRIS_BIN" "$ARCH")
  # makensis resolves a relative File path from the directory of iris.nsi.
  IRIS_BIN="$(cd "$(dirname "$IRIS_BIN")" && pwd)/$(basename "$IRIS_BIN")"
fi

if ! command -v makensis >/dev/null 2>&1; then
  echo "Error: building the setup program requires makensis (NSIS 3; Debian/Ubuntu: apt-get install nsis, Windows: choco install nsis)" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"
SETUP_NAME="iris-${VERSION}-windows-${ARCH}-setup.exe"
SETUP_PATH="$(cd "$OUT_DIR" && pwd)/$SETUP_NAME"
echo "Packaging $IRIS_BIN as $SETUP_PATH (version $VERSION_QUAD)"

LICENSE_DIR="$STAGING_DIR/licenses"
mkdir "$LICENSE_DIR"
iris_license_files "$REPO_ROOT" "$LICENSE_DIR"

# Under Git Bash, makensis finds no file for a File path in the
# C:/dir/file form MSYS converts arguments to; cygpath gives the
# C:\dir\file form. Elsewhere paths pass unchanged.
nsis_path() {
  if command -v cygpath >/dev/null 2>&1; then
    cygpath -w "$1"
  else
    printf '%s\n' "$1"
  fi
}

makensis \
  -DVERSION="$VERSION" \
  -DVERSION_QUAD="$VERSION_QUAD" \
  -DARCH="$ARCH" \
  -DBINARY_PATH="$(nsis_path "$IRIS_BIN")" \
  -DLICENSE_DIR="$(nsis_path "$LICENSE_DIR")" \
  -DICON_PATH="$(nsis_path "$REPO_ROOT/packaging/icons/iris.ico")" \
  -DOUTFILE="$(nsis_path "$SETUP_PATH")" \
  "$(nsis_path "$SCRIPT_DIR/iris.nsi")"

if [[ "$CHECK" == true ]]; then
  echo "iris.nsi compiles for $ARCH"
  exit 0
fi
if [[ "$GEN_SHA" == true ]]; then
  iris_sha256_sidecar "$SETUP_PATH"
fi
echo "Built $SETUP_PATH"
