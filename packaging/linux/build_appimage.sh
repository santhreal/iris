#!/usr/bin/env bash
# build_appimage.sh: builds the AppImage of iris with appimagetool.
#
#   packaging/linux/build_appimage.sh --bin <path> --out <dir>
#       [--version <ver>] [--arch x86_64|aarch64] [--skip-sha]
#       [--appimagetool <path>]
#
#   --bin <path>           the iris executable to package
#   --out <dir>            the directory the AppImage is written to
#   --version <ver>        the iris version (default: Cargo.toml)
#   --arch <arch>          x86_64 or aarch64 (default: the
#                          executable's); fails when the executable is
#                          built for another
#   --skip-sha             write no .sha256 sidecar
#   --appimagetool <path>  the appimagetool to run (default: appimagetool
#                          on PATH, else the continuous build for this
#                          machine, downloaded to .build-staging/)
#
# Writes iris-{ver}-linux-{arch}.AppImage (packaging/CONTRACT.md) and
# its .sha256 sidecar. The AppImage starts with the type2-runtime build
# for {arch}, downloaded from the continuous release of
# AppImage/type2-runtime, so an x86_64 machine can build the aarch64
# AppImage.
set -euo pipefail
# Packaged directories are 0755 and files 0644 whatever the caller's umask.
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
APPIMAGETOOL=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --bin) IRIS_BIN=${2:?--bin needs a path}; shift 2 ;;
    --out) OUT_DIR=${2:?--out needs a directory}; shift 2 ;;
    --version) VERSION=${2:?--version needs a version}; shift 2 ;;
    --arch) ARCH=${2:?--arch needs x86_64 or aarch64}; shift 2 ;;
    --skip-sha) GEN_SHA=false; shift ;;
    --appimagetool) APPIMAGETOOL=${2:?--appimagetool needs a path}; shift 2 ;;
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
APPIMAGE_NAME="iris-${VERSION}-linux-${ARCH}.AppImage"
APPIMAGE_PATH="$OUT_DIR/$APPIMAGE_NAME"
echo "Packaging $IRIS_BIN as $APPIMAGE_PATH"

TMP_DIR=$(iris_staging_dir "$REPO_ROOT" appimage)
trap 'rm -rf "$TMP_DIR"' EXIT

# fetch URL FILE: downloads URL to FILE.
fetch() {
  echo "Downloading $1"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 --connect-timeout 20 -o "$2" "$1"
  else
    wget -q --tries=3 --timeout=20 -O "$2" "$1"
  fi
}

# appimagetool runs on this machine: the build for this machine's
# architecture. The runtime runs where the AppImage runs: the build
# for $ARCH.
if [[ -z "$APPIMAGETOOL" ]]; then
  if command -v appimagetool >/dev/null 2>&1; then
    APPIMAGETOOL=appimagetool
  else
    APPIMAGETOOL="$REPO_ROOT/.build-staging/appimagetool-$(uname -m).AppImage"
    if [[ ! -x "$APPIMAGETOOL" ]]; then
      fetch "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$(uname -m).AppImage" \
        "$TMP_DIR/appimagetool"
      chmod 0755 "$TMP_DIR/appimagetool"
      mv "$TMP_DIR/appimagetool" "$APPIMAGETOOL"
    fi
  fi
fi
RUNTIME="$TMP_DIR/runtime-$ARCH"
fetch "https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-$ARCH" "$RUNTIME"

APPDIR="$TMP_DIR/iris.AppDir"
DOC_DIR="$APPDIR/usr/share/doc/iris"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/metainfo" "$DOC_DIR"
install -m 0755 "$IRIS_BIN" "$APPDIR/usr/bin/iris"
install -m 0755 "$SCRIPT_DIR/AppRun" "$APPDIR/AppRun"
install -m 0644 "$SCRIPT_DIR/dev.iris.app.desktop" "$APPDIR/"
install -m 0644 "$SCRIPT_DIR/dev.iris.app.desktop" "$APPDIR/usr/share/applications/"
install -m 0644 "$SCRIPT_DIR/dev.iris.app.metainfo.xml" "$APPDIR/usr/share/metainfo/"
iris_license_files "$REPO_ROOT" "$DOC_DIR"
chmod 0644 "$DOC_DIR"/*

install -m 0644 "$REPO_ROOT/packaging/icons/iris-256.png" "$APPDIR/iris.png"
ln -s iris.png "$APPDIR/.DirIcon"
iris_hicolor_icons "$REPO_ROOT" "$APPDIR/usr/share/icons/hicolor"

# NO_APPSTREAM=1: skip the AppStream validation, which needs network
# access. --appimage-extract-and-run: run appimagetool without FUSE.
NO_APPSTREAM=1 ARCH="$ARCH" "$APPIMAGETOOL" --appimage-extract-and-run \
  --runtime-file "$RUNTIME" "$APPDIR" "$APPIMAGE_PATH"
if [[ "$GEN_SHA" == true ]]; then
  iris_sha256_sidecar "$APPIMAGE_PATH"
fi
echo "Built $APPIMAGE_PATH"
