#!/usr/bin/env bash
# make_app.sh: assembles the iris.app bundle for macOS.
#
#   packaging/macos/make_app.sh --bin <path> --ffmpeg <dir> --out <dir>
#       [--version <ver>]
#
#   --bin <path>      the iris executable (universal: x86_64 and arm64)
#   --ffmpeg <dir>    universal runtime from build_ffmpeg.sh + lipo
#   --out <dir>       the directory iris.app is written to
#   --version <ver>   the iris version (default: Cargo.toml)
#
# Bundle layout:
#   iris.app/Contents/Info.plist      packaging/macos/Info.plist with
#                                     CFBundleShortVersionString and
#                                     CFBundleVersion set to the
#                                     major.minor.patch of {ver}, and
#                                     IrisVersion set to {ver}
#   iris.app/Contents/PkgInfo
#   iris.app/Contents/MacOS/iris
#   iris.app/Contents/MacOS/ffmpeg
#   iris.app/Contents/Resources/iris.icns
#   iris.app/Contents/Resources/{LICENSE-MIT,LICENSE-APACHE,Inter-OFL.txt}
#   iris.app/Contents/Resources/ffmpeg/{licenses and source locations}
#
# On macOS the bundle gets an ad-hoc code signature.
set -euo pipefail
# Bundle directories are 0755 and files 0644 whatever the caller's umask.
umask 022

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
# shellcheck source=packaging/lib.sh
. "$REPO_ROOT/packaging/lib.sh"

IRIS_BIN=""
FFMPEG_DIR=""
OUT_DIR=""
VERSION=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --bin) IRIS_BIN=${2:?--bin needs a path}; shift 2 ;;
    --ffmpeg) FFMPEG_DIR=${2:?--ffmpeg needs a directory}; shift 2 ;;
    --out) OUT_DIR=${2:?--out needs a directory}; shift 2 ;;
    --version) VERSION=${2:?--version needs a version}; shift 2 ;;
    -h|--help) iris_help "$0"; exit 0 ;;
    *) echo "Error: unknown argument $1 (see --help)" >&2; exit 2 ;;
  esac
done
if [[ -z "$IRIS_BIN" || -z "$FFMPEG_DIR" || -z "$OUT_DIR" ]]; then
  echo "Error: --bin, --ffmpeg, and --out are required (see --help)" >&2
  exit 2
fi
if [[ ! -f "$IRIS_BIN" ]]; then
  echo "Error: $IRIS_BIN does not exist" >&2
  exit 1
fi

VERSION=${VERSION:-$(iris_cargo_version "$REPO_ROOT")}
CORE_VERSION=$(iris_core_version "$VERSION")
FFMPEG_BIN=$(iris_ffmpeg_bin "$FFMPEG_DIR")

APP_DIR="$OUT_DIR/iris.app"
CONTENTS_DIR="$APP_DIR/Contents"
RESOURCES_DIR="$CONTENTS_DIR/Resources"
echo "Assembling $APP_DIR from $IRIS_BIN (version $VERSION, bundle version $CORE_VERSION)"

rm -rf "$APP_DIR"
mkdir -p "$CONTENTS_DIR/MacOS" "$RESOURCES_DIR/ffmpeg"
install -m 0755 "$IRIS_BIN" "$CONTENTS_DIR/MacOS/iris"
install -m 0755 "$FFMPEG_BIN" "$CONTENTS_DIR/MacOS/ffmpeg"
install -m 0644 "$FFMPEG_DIR/licenses/"* "$RESOURCES_DIR/ffmpeg/"
cp "$REPO_ROOT/packaging/icons/iris.icns" "$RESOURCES_DIR/iris.icns"
iris_license_files "$REPO_ROOT" "$RESOURCES_DIR"
sed -e "s/@IRIS_CORE_VERSION@/$CORE_VERSION/g" -e "s/@IRIS_VERSION@/$VERSION/g" \
  "$SCRIPT_DIR/Info.plist" >"$CONTENTS_DIR/Info.plist"
if grep -q '@IRIS_' "$CONTENTS_DIR/Info.plist"; then
  echo "Error: $CONTENTS_DIR/Info.plist holds a placeholder make_app.sh does not set" >&2
  exit 1
fi
if command -v plutil >/dev/null 2>&1; then
  plutil -lint "$CONTENTS_DIR/Info.plist" >/dev/null
fi
printf 'APPL????' >"$CONTENTS_DIR/PkgInfo"

if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep --sign - "$APP_DIR"
fi
echo "Built $APP_DIR"
