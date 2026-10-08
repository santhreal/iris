#!/usr/bin/env bash
# make_dmg.sh: builds the macOS disk image of iris from iris.app.
#
#   packaging/macos/make_dmg.sh --app <path> --out <dir>
#       [--version <ver>] [--skip-sha]
#
#   --app <path>      the iris.app bundle (make_app.sh)
#   --out <dir>       the directory the disk image is written to
#   --version <ver>   the iris version (default: Cargo.toml)
#   --skip-sha        write no .sha256 sidecar
#
# Writes iris-{ver}-macos-universal.dmg (packaging/CONTRACT.md) and its
# .sha256 sidecar. The image holds iris.app and an Applications link to
# /Applications. hdiutil (macOS) writes a compressed UDZO image;
# elsewhere genisoimage or mkisofs writes an ISO 9660 image with Apple
# extensions.
set -euo pipefail
# Staged directories are 0755 and files 0644 whatever the caller's umask.
umask 022

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
# shellcheck source=packaging/lib.sh
. "$REPO_ROOT/packaging/lib.sh"

APP_PATH=""
OUT_DIR=""
VERSION=""
GEN_SHA=true
while [[ $# -gt 0 ]]; do
  case "$1" in
    --app) APP_PATH=${2:?--app needs a path}; shift 2 ;;
    --out) OUT_DIR=${2:?--out needs a directory}; shift 2 ;;
    --version) VERSION=${2:?--version needs a version}; shift 2 ;;
    --skip-sha) GEN_SHA=false; shift ;;
    -h|--help) iris_help "$0"; exit 0 ;;
    *) echo "Error: unknown argument $1 (see --help)" >&2; exit 2 ;;
  esac
done
if [[ -z "$APP_PATH" || -z "$OUT_DIR" ]]; then
  echo "Error: --app and --out are required (see --help)" >&2
  exit 2
fi
if [[ ! -f "$APP_PATH/Contents/MacOS/iris" ]]; then
  echo "Error: $APP_PATH is not an iris.app bundle (no Contents/MacOS/iris); build it with make_app.sh" >&2
  exit 1
fi

VERSION=${VERSION:-$(iris_cargo_version "$REPO_ROOT")}
mkdir -p "$OUT_DIR"
DMG_NAME="iris-${VERSION}-macos-universal.dmg"
DMG_PATH="$OUT_DIR/$DMG_NAME"
echo "Packaging $APP_PATH as $DMG_PATH"
rm -f "$DMG_PATH" "$DMG_PATH.sha256"

STAGING_DIR=$(iris_staging_dir "$REPO_ROOT" dmg)
trap 'rm -rf "$STAGING_DIR"' EXIT
# The staging directory is the volume root; mktemp creates it 0700.
chmod 0755 "$STAGING_DIR"
cp -R "$APP_PATH" "$STAGING_DIR/iris.app"
ln -s /Applications "$STAGING_DIR/Applications"
cp "$REPO_ROOT/packaging/icons/iris.icns" "$STAGING_DIR/.VolumeIcon.icns"

if command -v hdiutil >/dev/null 2>&1; then
  if command -v SetFile >/dev/null 2>&1; then
    SetFile -a C "$STAGING_DIR" 2>/dev/null || true
  fi
  hdiutil create -volname iris -srcfolder "$STAGING_DIR" -ov -format UDZO "$DMG_PATH"
elif command -v genisoimage >/dev/null 2>&1; then
  genisoimage -V iris -D -R -apple -no-pad -o "$DMG_PATH" "$STAGING_DIR"
elif command -v mkisofs >/dev/null 2>&1; then
  mkisofs -V iris -D -R -apple -no-pad -o "$DMG_PATH" "$STAGING_DIR"
else
  echo "Error: building the disk image requires hdiutil (macOS), genisoimage or mkisofs" >&2
  exit 1
fi
if [[ "$GEN_SHA" == true ]]; then
  iris_sha256_sidecar "$DMG_PATH"
fi
echo "Built $DMG_PATH"
