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
#   --appimagetool <path>  the appimagetool to run (default: the pinned
#                          release for this machine, downloaded to
#                          .build-staging/ and checked against its
#                          recorded SHA-256)
#
# Writes iris-{ver}-linux-{arch}.AppImage (packaging/CONTRACT.md) and
# its .sha256 sidecar. The AppImage starts with the pinned type2-runtime
# release for {arch}, checked against its recorded SHA-256, so an x86_64
# machine can build the aarch64 AppImage.
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

# Pinned tool releases. Change a version and its hashes together. Each
# hash was taken from a download whose signature verified: type2-runtime
# signs every runtime with key 570C77ACEA40C0F1B758902CBF96CCA56490F695
# (runtime-{arch}.sig on the release).
APPIMAGETOOL_VERSION=1.9.1
RUNTIME_VERSION=20251108

# pinned_sha256 NAME: prints the recorded SHA-256 of the pinned file NAME.
pinned_sha256() {
  case "$1" in
    appimagetool-x86_64.AppImage) echo ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0 ;;
    appimagetool-aarch64.AppImage) echo f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158 ;;
    runtime-x86_64) echo 2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d ;;
    runtime-aarch64) echo 00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444 ;;
    *) echo "Error: no pinned $1 exists; build on an x86_64 or aarch64 machine" >&2; return 1 ;;
  esac
}

# pinned_ok NAME FILE: whether FILE has the SHA-256 recorded for NAME.
pinned_ok() {
  local want
  want=$(pinned_sha256 "$1") || return 1
  [[ "$(sha256sum "$2" | cut -d' ' -f1)" == "$want" ]]
}

# fetch_pinned URL FILE: downloads URL to FILE. Deletes FILE and fails
# unless its SHA-256 is the one recorded for the URL's file name.
fetch_pinned() {
  local name=${1##*/}
  pinned_sha256 "$name" >/dev/null
  echo "Downloading $1"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 --connect-timeout 20 -o "$2" "$1"
  else
    wget -q --tries=3 --timeout=20 -O "$2" "$1"
  fi
  if ! pinned_ok "$name" "$2"; then
    echo "Error: $1 has SHA-256 $(sha256sum "$2" | cut -d' ' -f1), not the pinned $(pinned_sha256 "$name"); deleted it" >&2
    rm -f "$2"
    return 1
  fi
}

# appimagetool runs on this machine: the build for this machine's
# architecture. The runtime runs where the AppImage runs: the build
# for $ARCH.
if [[ -z "$APPIMAGETOOL" ]]; then
  TOOL_NAME="appimagetool-$(uname -m).AppImage"
  APPIMAGETOOL="$REPO_ROOT/.build-staging/appimagetool-$APPIMAGETOOL_VERSION-$(uname -m).AppImage"
  if [[ ! -x "$APPIMAGETOOL" ]] || ! pinned_ok "$TOOL_NAME" "$APPIMAGETOOL"; then
    fetch_pinned "https://github.com/AppImage/appimagetool/releases/download/$APPIMAGETOOL_VERSION/$TOOL_NAME" \
      "$TMP_DIR/appimagetool"
    chmod 0755 "$TMP_DIR/appimagetool"
    mv "$TMP_DIR/appimagetool" "$APPIMAGETOOL"
  fi
fi
RUNTIME="$TMP_DIR/runtime-$ARCH"
fetch_pinned "https://github.com/AppImage/type2-runtime/releases/download/$RUNTIME_VERSION/runtime-$ARCH" "$RUNTIME"

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
