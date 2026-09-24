#!/usr/bin/env bash
set -euo pipefail

# build_appimage.sh — Builds an AppImage for iris
#
# References:
#   packaging/CONTRACT.md
#
# Release asset name:
#   iris-{ver}-linux-x86_64.AppImage
#
# Usage:
#   ./build_appimage.sh [options] [binary-path] [output-dir]
#
# Options:
#   -b, --bin <path>            Path to iris executable binary
#   -o, --out <dir>             Output directory for .AppImage (default: dist)
#   -v, --version <ver>         App version (default: from Cargo.toml)
#   -a, --arch <arch>           Target architecture (default: x86_64)
#   --appimagetool <path>       Custom path to appimagetool executable
#   -s, --skip-sha              Skip generating .sha256 checksum sidecar
#   -h, --help                  Show this help message

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

show_help() {
  sed -ne '/^#/!q;s/^# //;2,$p' "$0"
  exit 0
}

IRIS_BIN=""
OUT_DIR=""
VERSION=""
ARCH="x86_64"
CUSTOM_APPIMAGETOOL=""
GEN_SHA=true

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
    -a|--arch)
      ARCH="$2"
      shift 2
      ;;
    --appimagetool)
      CUSTOM_APPIMAGETOOL="$2"
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
      show_help
      ;;
    *)
      if [[ -z "$IRIS_BIN" ]]; then
        IRIS_BIN="$1"
      elif [[ -z "$OUT_DIR" ]]; then
        OUT_DIR="$1"
      else
        echo "Error: Unexpected argument $1" >&2
        show_help
      fi
      shift
      ;;
  esac
done

# Resolve version from Cargo.toml if not specified
if [[ -z "$VERSION" ]]; then
  if [[ -f "$REPO_ROOT/Cargo.toml" ]]; then
    VERSION=$(grep -m1 '^version = ' "$REPO_ROOT/Cargo.toml" | cut -d '"' -f2)
  else
    VERSION="0.1.0"
  fi
fi

# Resolve output directory
if [[ -z "$OUT_DIR" ]]; then
  OUT_DIR="$REPO_ROOT/dist"
fi
mkdir -p "$OUT_DIR"

# Locate the iris release binary in cargo's configured target directory.
if [[ -z "$IRIS_BIN" ]]; then
  TARGET_DIR=$(cd "$REPO_ROOT" && cargo metadata --format-version 1 --no-deps \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
  IRIS_BIN="$TARGET_DIR/release/iris"
  if [[ ! -x "$IRIS_BIN" ]]; then
    echo "No release binary at $IRIS_BIN. Building with cargo..."
    (cd "$REPO_ROOT" && cargo build --release --locked)
  fi
fi

if [[ ! -x "$IRIS_BIN" ]]; then
  echo "Error: '$IRIS_BIN' is not executable." >&2
  exit 1
fi

echo "Using iris binary: $IRIS_BIN"
echo "Packaging iris AppImage version: $VERSION ($ARCH)"

# Staging directory inside the repository (.build-staging is gitignored).
STAGING_BASE="$REPO_ROOT/.build-staging"
mkdir -p "$STAGING_BASE"
APPDIR="$(mktemp -d "$STAGING_BASE/appimage_appdir.XXXXXX")"
trap 'rm -rf "$APPDIR"' EXIT

# Create standard AppDir hierarchy
mkdir -p "$APPDIR/usr/bin"
mkdir -p "$APPDIR/usr/share/applications"
mkdir -p "$APPDIR/usr/share/icons/hicolor"

# Install iris binary
cp "$IRIS_BIN" "$APPDIR/usr/bin/iris"
chmod 0755 "$APPDIR/usr/bin/iris"
if command -v strip >/dev/null 2>&1; then
  strip --strip-unneeded "$APPDIR/usr/bin/iris" 2>/dev/null || true
fi

# Install AppRun
cp "$SCRIPT_DIR/AppRun" "$APPDIR/AppRun"
chmod 0755 "$APPDIR/AppRun"

# Install desktop file at root and in usr/share/applications
cp "$SCRIPT_DIR/iris.desktop" "$APPDIR/iris.desktop"
chmod 0644 "$APPDIR/iris.desktop"

cp "$SCRIPT_DIR/iris.desktop" "$APPDIR/usr/share/applications/iris.desktop"
chmod 0644 "$APPDIR/usr/share/applications/iris.desktop"

# Install AppStream metainfo
if [[ -f "$SCRIPT_DIR/dev.iris.app.metainfo.xml" ]]; then
  mkdir -p "$APPDIR/usr/share/metainfo"
  cp "$SCRIPT_DIR/dev.iris.app.metainfo.xml" "$APPDIR/usr/share/metainfo/dev.iris.app.metainfo.xml"
  chmod 0644 "$APPDIR/usr/share/metainfo/"*
fi

# Install icons
BASE_ICON="$REPO_ROOT/packaging/icons/iris-1024.png"
if [[ ! -f "$BASE_ICON" ]]; then
  BASE_ICON="$REPO_ROOT/packaging/icons/iris-256.png"
fi

# Root icon (iris.png) and .DirIcon symlink
if [[ -f "$REPO_ROOT/packaging/icons/iris-256.png" ]]; then
  cp "$REPO_ROOT/packaging/icons/iris-256.png" "$APPDIR/iris.png"
elif [[ -f "$BASE_ICON" ]]; then
  cp "$BASE_ICON" "$APPDIR/iris.png"
fi
chmod 0644 "$APPDIR/iris.png"
(cd "$APPDIR" && ln -sf iris.png .DirIcon)

# Populate hicolor icon theme
if command -v python3 >/dev/null 2>&1 && python3 -c "import PIL" >/dev/null 2>&1 && [[ -f "$BASE_ICON" ]]; then
  python3 - <<EOF
from PIL import Image
import os

base = Image.open("$BASE_ICON")
app_dir = "$APPDIR"

sizes = [16, 24, 32, 48, 64, 128, 256, 512, 1024]
for s in sizes:
    dest_dir = os.path.join(app_dir, "usr/share/icons/hicolor", f"{s}x{s}", "apps")
    os.makedirs(dest_dir, exist_ok=True)
    resized = base.resize((s, s), Image.LANCZOS)
    resized.save(os.path.join(dest_dir, "iris.png"), "PNG")
EOF
else
  for s in 256 512 1024; do
    src="$REPO_ROOT/packaging/icons/iris-${s}.png"
    if [[ -f "$src" ]]; then
      dest="$APPDIR/usr/share/icons/hicolor/${s}x${s}/apps"
      mkdir -p "$dest"
      cp "$src" "$dest/iris.png"
      chmod 0644 "$dest/iris.png"
    fi
  done
fi

# Locate or acquire appimagetool
APPIMAGETOOL=""
if [[ -n "$CUSTOM_APPIMAGETOOL" && -x "$CUSTOM_APPIMAGETOOL" ]]; then
  APPIMAGETOOL="$CUSTOM_APPIMAGETOOL"
elif command -v appimagetool >/dev/null 2>&1; then
  APPIMAGETOOL="appimagetool"
elif [[ -x "$REPO_ROOT/packaging/linux/appimagetool-x86_64.AppImage" ]]; then
  APPIMAGETOOL="$REPO_ROOT/packaging/linux/appimagetool-x86_64.AppImage"
elif [[ -x "$STAGING_BASE/appimagetool" ]]; then
  APPIMAGETOOL="$STAGING_BASE/appimagetool"
fi

# Try downloading appimagetool if not found
if [[ -z "$APPIMAGETOOL" ]]; then
  # appimagetool runs on the build host, so fetch the host's build of it.
  DOWNLOAD_URL="https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$(uname -m).AppImage"
  CACHED_TOOL="$STAGING_BASE/appimagetool"
  echo "appimagetool not found locally; attempting to download from AppImage GitHub..."
  DOWNLOAD_OK=false
  if command -v curl >/dev/null 2>&1; then
    if curl -fsSL --connect-timeout 10 -o "$CACHED_TOOL" "$DOWNLOAD_URL" 2>/dev/null; then
      chmod +x "$CACHED_TOOL"
      DOWNLOAD_OK=true
    fi
  elif command -v wget >/dev/null 2>&1; then
    if wget -q --timeout=10 -O "$CACHED_TOOL" "$DOWNLOAD_URL" 2>/dev/null; then
      chmod +x "$CACHED_TOOL"
      DOWNLOAD_OK=true
    fi
  fi

  if [[ "$DOWNLOAD_OK" == true && -x "$CACHED_TOOL" ]]; then
    APPIMAGETOOL="$CACHED_TOOL"
    echo "Downloaded appimagetool to $CACHED_TOOL"
  fi
fi

# Target AppImage filename per packaging/CONTRACT.md:
# iris-{ver}-linux-x86_64.AppImage
CONTRACT_APPIMAGE="iris-${VERSION}-linux-${ARCH}.AppImage"
CONTRACT_APPIMAGE_PATH="$OUT_DIR/$CONTRACT_APPIMAGE"

if [[ -z "$APPIMAGETOOL" ]]; then
  echo "Error: appimagetool is not installed and the download failed." >&2
  echo "Install it, or pass its path: $0 --appimagetool /path/to/appimagetool" >&2
  exit 1
fi

echo "Building AppImage using appimagetool ($APPIMAGETOOL)..."
# NO_APPSTREAM=1: skip the AppStream validation that needs network access.
# --appimage-extract-and-run: run appimagetool without FUSE.
NO_APPSTREAM=1 ARCH="$ARCH" "$APPIMAGETOOL" --appimage-extract-and-run "$APPDIR" "$CONTRACT_APPIMAGE_PATH"

# Generate SHA256 checksum sidecar
if [[ "$GEN_SHA" == true && -f "$CONTRACT_APPIMAGE_PATH" ]]; then
  (cd "$OUT_DIR" && sha256sum "$CONTRACT_APPIMAGE" > "$CONTRACT_APPIMAGE.sha256")
  echo "Created checksum: $CONTRACT_APPIMAGE_PATH.sha256"
fi

echo "Successfully built AppImage:"
echo "  $CONTRACT_APPIMAGE_PATH"
