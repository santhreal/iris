#!/usr/bin/env bash
set -euo pipefail

# make_app.sh — Assembles the iris.app macOS application bundle
#
# References:
#   packaging/CONTRACT.md
#
# Bundle Structure:
#   iris.app/
#     Contents/
#       Info.plist
#       PkgInfo
#       MacOS/
#         iris (executable)
#       Resources/
#         iris.icns (application icon)
#
# Usage:
#   ./make_app.sh [options] [binary-path] [output-dir]
#
# Options:
#   -b, --bin <path>      Path to iris executable binary
#   -o, --out <dir>       Output directory where iris.app will be created (default: dist)
#   -v, --version <ver>   App version (default: from Cargo.toml)
#   -h, --help            Show this help message

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

show_help() {
  sed -ne '/^#/!q;s/^# //;2,$p' "$0"
  exit 0
}

IRIS_BIN=""
OUT_DIR=""
VERSION=""

# Parse flags and arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    -b|--bin)
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
      show_help
      ;;
    -*)
      echo "Error: Unknown option $1" >&2
      exit 1
      ;;
    *)
      if [[ -z "$IRIS_BIN" ]]; then
        IRIS_BIN="$1"
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

# 1. Locate iris binary if not explicitly provided
if [[ -z "$IRIS_BIN" ]]; then
  SEARCH_PATHS=(
    "$REPO_ROOT/target/universal-apple-darwin/release/iris"
    "$REPO_ROOT/target/aarch64-apple-darwin/release/iris"
    "$REPO_ROOT/target/x86_64-apple-darwin/release/iris"
    "$REPO_ROOT/target/release/iris"
    "$REPO_ROOT/target/debug/iris"
  )
  for candidate in "${SEARCH_PATHS[@]}"; do
    if [[ -f "$candidate" && -x "$candidate" ]]; then
      IRIS_BIN="$candidate"
      break
    fi
  done
fi

if [[ -z "$IRIS_BIN" || ! -f "$IRIS_BIN" ]]; then
  echo "Error: iris executable binary not found." >&2
  if [[ -n "$IRIS_BIN" ]]; then
    echo "  Specified path does not exist: $IRIS_BIN" >&2
  else
    echo "  Looked in:" >&2
    for candidate in "${SEARCH_PATHS[@]}"; do
      echo "    - $candidate" >&2
    done
  fi
  echo "" >&2
  echo "Please build the binary first or specify its location:" >&2
  echo "  $0 --bin <path-to-iris-binary> [--out <output-dir>]" >&2
  exit 1
fi

# 2. Determine version
if [[ -z "$VERSION" ]]; then
  if [[ -f "$REPO_ROOT/Cargo.toml" ]]; then
    VERSION=$(grep -m1 '^version = ' "$REPO_ROOT/Cargo.toml" | sed -E 's/version = "([^"]+)"/\1/')
  fi
fi
VERSION="${VERSION:-0.1.0}"

# 3. Prepare target bundle structure
APP_DIR="$OUT_DIR/iris.app"
CONTENTS_DIR="$APP_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"

echo "==> Assembling iris.app"
echo "    Binary:  $IRIS_BIN"
echo "    Version: $VERSION"
echo "    Target:  $APP_DIR"

rm -rf "$APP_DIR"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"

# 4. Copy binary
cp "$IRIS_BIN" "$MACOS_DIR/iris"
chmod +x "$MACOS_DIR/iris"

# 5. Copy application icon
ICON_SRC="$REPO_ROOT/packaging/icons/iris.icns"
if [[ ! -f "$ICON_SRC" ]]; then
  echo "Error: Icon file missing at $ICON_SRC" >&2
  exit 1
fi
cp "$ICON_SRC" "$RESOURCES_DIR/iris.icns"

# 6. Generate Info.plist with injected version
PLIST_SRC="$SCRIPT_DIR/Info.plist"
if [[ ! -f "$PLIST_SRC" ]]; then
  echo "Error: Info.plist template missing at $PLIST_SRC" >&2
  exit 1
fi

sed -E \
  -e "/<key>CFBundleShortVersionString<\/key>/ { n; s|<string>[^<]*</string>|<string>${VERSION}</string>|; }" \
  -e "/<key>CFBundleVersion<\/key>/ { n; s|<string>[^<]*</string>|<string>${VERSION}</string>|; }" \
  "$PLIST_SRC" > "$CONTENTS_DIR/Info.plist"

if command -v plutil >/dev/null 2>&1; then
  plutil -replace CFBundleShortVersionString -string "$VERSION" "$CONTENTS_DIR/Info.plist"
  plutil -replace CFBundleVersion -string "$VERSION" "$CONTENTS_DIR/Info.plist"
  plutil -lint "$CONTENTS_DIR/Info.plist" >/dev/null
fi

# 7. Write PkgInfo
printf "APPL????" > "$CONTENTS_DIR/PkgInfo"

# 8. Ad-hoc codesign if running on macOS with codesign available
if command -v codesign >/dev/null 2>&1; then
  echo "==> Applying ad-hoc code signature..."
  codesign --force --deep --sign - "$APP_DIR"
fi

echo "==> Created successfully: $APP_DIR"
