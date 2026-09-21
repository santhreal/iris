#!/usr/bin/env bash
set -euo pipefail

# make_dmg.sh — Creates a compressed macOS disk image (.dmg) for iris
#
# References:
#   packaging/CONTRACT.md
#
# Release asset name:
#   iris-{ver}-macos-universal.dmg (or user-specified name)
#
# Contents of DMG:
#   iris.app/
#   Applications -> /Applications (drag-and-drop installation symlink)
#
# Usage:
#   ./make_dmg.sh [options] [path-to-iris.app] [output-dir]
#
# Options:
#   -a, --app <path>      Path to iris.app bundle
#   -o, --out <dir>       Output directory for .dmg (default: dist)
#   -n, --name <name>     Custom DMG filename (default: iris-{ver}-macos-universal.dmg)
#   -v, --version <ver>   App version (default: from Cargo.toml)
#   -s, --skip-sha        Skip generating .sha256 checksum sidecar
#   -h, --help            Show this help message

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

show_help() {
  sed -ne '/^#/!q;s/^# //;2,$p' "$0"
  exit 0
}

APP_PATH=""
OUT_DIR=""
DMG_NAME=""
VERSION=""
GEN_SHA=true

# Parse flags and arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    -a|--app)
      APP_PATH="$2"
      shift 2
      ;;
    -o|--out)
      OUT_DIR="$2"
      shift 2
      ;;
    -n|--name)
      DMG_NAME="$2"
      shift 2
      ;;
    -v|--version)
      VERSION="$2"
      shift 2
      ;;
    -s|--skip-sha)
      GEN_SHA=false
      shift
      ;;
    -h|--help)
      show_help
      ;;
    -*)
      echo "Error: Unknown option $1" >&2
      exit 1
      ;;
    *)
      if [[ -z "$APP_PATH" ]]; then
        APP_PATH="$1"
      elif [[ -z "$OUT_DIR" ]]; then
        OUT_DIR="$1"
      else
        echo "Error: Unexpected argument $1" >&2
        exit 1
      fi
      shift
      ;;
  esac
done

OUT_DIR="${OUT_DIR:-$REPO_ROOT/dist}"

# 1. Determine version
if [[ -z "$VERSION" ]]; then
  if [[ -f "$REPO_ROOT/Cargo.toml" ]]; then
    VERSION=$(grep -m1 '^version = ' "$REPO_ROOT/Cargo.toml" | sed -E 's/version = "([^"]+)"/\1/')
  fi
fi
VERSION="${VERSION:-0.1.0}"

# 2. Locate or assemble iris.app
if [[ -z "$APP_PATH" ]]; then
  CANDIDATES=(
    "$OUT_DIR/iris.app"
    "$REPO_ROOT/dist/iris.app"
    "$REPO_ROOT/target/macos/iris.app"
  )
  for candidate in "${CANDIDATES[@]}"; do
    if [[ -d "$candidate" && -f "$candidate/Contents/MacOS/iris" ]]; then
      APP_PATH="$candidate"
      break
    fi
  done
fi

if [[ -z "$APP_PATH" || ! -d "$APP_PATH" ]]; then
  if [[ -f "$SCRIPT_DIR/make_app.sh" ]]; then
    echo "==> iris.app not found. Attempting to assemble with make_app.sh..."
    bash "$SCRIPT_DIR/make_app.sh" --out "$OUT_DIR" --version "$VERSION"
    APP_PATH="$OUT_DIR/iris.app"
  fi
fi

if [[ -z "$APP_PATH" || ! -d "$APP_PATH" || ! -f "$APP_PATH/Contents/MacOS/iris" ]]; then
  echo "Error: Valid iris.app bundle not found." >&2
  if [[ -n "$APP_PATH" ]]; then
    echo "  Specified path is not a valid bundle: $APP_PATH" >&2
  fi
  echo "Please build iris.app first using make_app.sh:" >&2
  echo "  $SCRIPT_DIR/make_app.sh [path-to-binary]" >&2
  exit 1
fi

# 3. Determine DMG filename
# CONTRACT.md specifies: iris-{ver}-macos-universal.dmg
DMG_NAME="${DMG_NAME:-iris-${VERSION}-macos-universal.dmg}"
if [[ "$DMG_NAME" != *.dmg ]]; then
  DMG_NAME="${DMG_NAME}.dmg"
fi

mkdir -p "$OUT_DIR"
DMG_PATH="$OUT_DIR/$DMG_NAME"

echo "==> Creating macOS Disk Image"
echo "    Source App: $APP_PATH"
echo "    Output DMG: $DMG_PATH"

# 4. Prepare temporary staging directory
STAGING_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t 'iris_dmg_staging')"
cleanup() {
  rm -rf "$STAGING_DIR"
}
trap cleanup EXIT

echo "==> Populating staging folder..."
cp -R "$APP_PATH" "$STAGING_DIR/iris.app"

# Add /Applications symlink for drag-and-drop installation
ln -s /Applications "$STAGING_DIR/Applications"

# Include volume icon if available
ICON_SRC="$REPO_ROOT/packaging/icons/iris.icns"
if [[ -f "$ICON_SRC" ]]; then
  cp "$ICON_SRC" "$STAGING_DIR/.VolumeIcon.icns"
fi

# Remove any pre-existing output DMG and checksum
rm -f "$DMG_PATH" "${DMG_PATH}.sha256"

# 5. Build compressed disk image
if command -v hdiutil >/dev/null 2>&1; then
  echo "==> Building compressed DMG using hdiutil (UDZO)..."
  if [[ -f "$STAGING_DIR/.VolumeIcon.icns" ]] && command -v SetFile >/dev/null 2>&1; then
    SetFile -a C "$STAGING_DIR" 2>/dev/null || true
  fi

  hdiutil create \
    -volname "iris" \
    -srcfolder "$STAGING_DIR" \
    -ov \
    -format UDZO \
    "$DMG_PATH"
elif command -v genisoimage >/dev/null 2>&1; then
  echo "==> hdiutil not found; generating DMG image with genisoimage (Apple HFS extensions)..."
  genisoimage \
    -V "iris" \
    -D -R -apple \
    -no-pad \
    -file-mode 0755 \
    -o "$DMG_PATH" \
    "$STAGING_DIR"
elif command -v mkisofs >/dev/null 2>&1; then
  echo "==> hdiutil not found; generating DMG image with mkisofs (Apple HFS extensions)..."
  mkisofs \
    -V "iris" \
    -D -R -apple \
    -no-pad \
    -file-mode 0755 \
    -o "$DMG_PATH" \
    "$STAGING_DIR"
else
  echo "Error: Disk image creation requires hdiutil (on macOS) or genisoimage/mkisofs." >&2
  exit 1
fi

# 6. Generate SHA-256 sidecar (matching CONTRACT.md: each asset + .sha256 sidecar)
if [[ "$GEN_SHA" == "true" ]]; then
  echo "==> Generating SHA-256 checksum sidecar..."
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$OUT_DIR" && sha256sum "$DMG_NAME" > "${DMG_NAME}.sha256")
  elif command -v shasum >/dev/null 2>&1; then
    (cd "$OUT_DIR" && shasum -a 256 "$DMG_NAME" > "${DMG_NAME}.sha256")
  fi
  if [[ -f "${DMG_PATH}.sha256" ]]; then
    echo "==> Created checksum: ${DMG_PATH}.sha256"
  fi
fi

echo "==> Created successfully: $DMG_PATH"
