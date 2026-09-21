#!/usr/bin/env bash
set -euo pipefail

# build_deb.sh — Builds a Debian (.deb) package for iris
#
# References:
#   packaging/CONTRACT.md
#
# Release asset name:
#   iris-{ver}-linux-x86_64.deb
#
# Usage:
#   ./build_deb.sh [options] [binary-path] [output-dir]
#
# Options:
#   -b, --bin <path>      Path to iris executable binary
#   -o, --out <dir>       Output directory for .deb (default: dist)
#   -v, --version <ver>   App version (default: from Cargo.toml)
#   -a, --arch <arch>     Target architecture (default: x86_64)
#   -s, --skip-sha        Skip generating .sha256 checksum sidecar
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
ARCH="x86_64"
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

# Map architecture name to Debian architecture
DEB_ARCH="amd64"
case "$ARCH" in
  x86_64|amd64)
    DEB_ARCH="amd64"
    ARCH="x86_64"
    ;;
  aarch64|arm64)
    DEB_ARCH="arm64"
    ARCH="aarch64"
    ;;
  *)
    DEB_ARCH="$ARCH"
    ;;
esac

# Check for dpkg-deb
if ! command -v dpkg-deb >/dev/null 2>&1; then
  echo "Error: 'dpkg-deb' is required to build Debian packages but was not found." >&2
  exit 1
fi

# Locate iris executable binary
if [[ -z "$IRIS_BIN" ]]; then
  CANDIDATES=(
    "$REPO_ROOT/target/release/iris"
    "target/release/iris"
    "target/release/iris"
  )
  for cand in "${CANDIDATES[@]}"; do
    if [[ -x "$cand" ]]; then
      IRIS_BIN="$cand"
      break
    fi
  done
fi

if [[ -z "$IRIS_BIN" || ! -f "$IRIS_BIN" ]]; then
  echo "No pre-built iris binary found. Building with cargo..."
  (cd "$REPO_ROOT" && cargo build --release)
  if [[ -x "$REPO_ROOT/target/release/iris" ]]; then
    IRIS_BIN="$REPO_ROOT/target/release/iris"
  else
    # Try cargo target dir
    TARGET_FOUND=$(find target -name iris -type f -perm -111 2>/dev/null | grep release/iris | head -n 1 || true)
    if [[ -n "$TARGET_FOUND" && -x "$TARGET_FOUND" ]]; then
      IRIS_BIN="$TARGET_FOUND"
    else
      echo "Error: Failed to find or build iris binary" >&2
      exit 1
    fi
  fi
fi

if [[ ! -x "$IRIS_BIN" ]]; then
  echo "Error: '$IRIS_BIN' is not executable." >&2
  exit 1
fi

echo "Using iris binary: $IRIS_BIN"
echo "Packaging iris version: $VERSION ($DEB_ARCH)"

# Create temporary staging directory
STAGING_BASE="$REPO_ROOT/.build-staging"
mkdir -p "$STAGING_BASE"
TMP_DIR="$(mktemp -d "$STAGING_BASE/deb_build.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

PKG_DIR="$TMP_DIR/pkg"
mkdir -p "$PKG_DIR"

# Standard Debian package filesystem layout
mkdir -p "$PKG_DIR/usr/bin"
mkdir -p "$PKG_DIR/usr/share/applications"
mkdir -p "$PKG_DIR/etc/xdg/autostart"
mkdir -p "$PKG_DIR/usr/share/doc/iris"
mkdir -p "$PKG_DIR/DEBIAN"

# Install binary (strip to remove debug symbols if not already stripped)
cp "$IRIS_BIN" "$PKG_DIR/usr/bin/iris"
chmod 0755 "$PKG_DIR/usr/bin/iris"
if command -v strip >/dev/null 2>&1; then
  strip --strip-unneeded "$PKG_DIR/usr/bin/iris" 2>/dev/null || true
fi

# Install desktop files
cp "$SCRIPT_DIR/iris.desktop" "$PKG_DIR/usr/share/applications/iris.desktop"
chmod 0644 "$PKG_DIR/usr/share/applications/iris.desktop"

cp "$SCRIPT_DIR/iris-autostart.desktop" "$PKG_DIR/etc/xdg/autostart/iris-autostart.desktop"
chmod 0644 "$PKG_DIR/etc/xdg/autostart/iris-autostart.desktop"

# Install AppStream metainfo
if [[ -f "$SCRIPT_DIR/dev.iris.app.metainfo.xml" ]]; then
  mkdir -p "$PKG_DIR/usr/share/metainfo"
  cp "$SCRIPT_DIR/dev.iris.app.metainfo.xml" "$PKG_DIR/usr/share/metainfo/dev.iris.app.metainfo.xml"
  chmod 0644 "$PKG_DIR/usr/share/metainfo/dev.iris.app.metainfo.xml"
fi

# Install icons across standard resolutions
ICON_SIZES=(16 24 32 48 64 128 256 512 1024)
BASE_ICON="$REPO_ROOT/packaging/icons/iris-1024.png"
if [[ ! -f "$BASE_ICON" ]]; then
  BASE_ICON="$REPO_ROOT/packaging/icons/iris-256.png"
fi

# Generate / copy icons
if command -v python3 >/dev/null 2>&1 && python3 -c "import PIL" >/dev/null 2>&1 && [[ -f "$BASE_ICON" ]]; then
  python3 - <<EOF
from PIL import Image
import os

base = Image.open("$BASE_ICON")
pkg_dir = "$PKG_DIR"

sizes = [16, 24, 32, 48, 64, 128, 256, 512, 1024]
for s in sizes:
    dest_dir = os.path.join(pkg_dir, "usr/share/icons/hicolor", f"{s}x{s}", "apps")
    os.makedirs(dest_dir, exist_ok=True)
    resized = base.resize((s, s), Image.LANCZOS)
    resized.save(os.path.join(dest_dir, "iris.png"), "PNG")
EOF
else
  # Fallback: copy available pre-rendered icons
  for s in 256 512 1024; do
    src="$REPO_ROOT/packaging/icons/iris-${s}.png"
    if [[ -f "$src" ]]; then
      dest="$PKG_DIR/usr/share/icons/hicolor/${s}x${s}/apps"
      mkdir -p "$dest"
      cp "$src" "$dest/iris.png"
      chmod 0644 "$dest/iris.png"
    fi
  done
fi

# Install copyright notice
cat > "$PKG_DIR/usr/share/doc/iris/copyright" <<EOF
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: iris
Upstream-Contact: Santh <64453045+santhreal@users.noreply.github.com>
Source: https://github.com/santhreal/iris

Files: *
Copyright: 2026 Santh <64453045+santhreal@users.noreply.github.com>
License: MIT or Apache-2.0
EOF
chmod 0644 "$PKG_DIR/usr/share/doc/iris/copyright"

# Calculate Installed-Size in KiB
INSTALLED_SIZE=$(du -sk "$PKG_DIR" | cut -f1)

# Generate DEBIAN/control
cat > "$PKG_DIR/DEBIAN/control" <<EOF
Package: iris
Version: ${VERSION}
Section: utils
Priority: optional
Architecture: ${DEB_ARCH}
Maintainer: Santh <64453045+santhreal@users.noreply.github.com>
Installed-Size: ${INSTALLED_SIZE}
Homepage: https://github.com/santhreal/iris
Depends: libc6 (>= 2.34), libpipewire-0.3-0 (>= 0.3.0), libxkbcommon0, libxkbcommon-x11-0, libxcb1, libxcb-xkb1, libfontconfig1, libwayland-client0, libx11-6
Recommends: wl-clipboard | xclip
Description: Screenshot and screen-recording utility
 iris is a native, lightweight screen capture and screen recording
 utility built with GPUI. It runs a background daemon with tray
 controls, global hotkeys, and an IPC listener, paired with region
 capture, window capture, screen recording via PipeWire, and an
 annotation editor.
EOF
chmod 0644 "$PKG_DIR/DEBIAN/control"

# Install control scripts (postinst, postrm)
cp "$SCRIPT_DIR/deb/postinst" "$PKG_DIR/DEBIAN/postinst"
chmod 0755 "$PKG_DIR/DEBIAN/postinst"

cp "$SCRIPT_DIR/deb/postrm" "$PKG_DIR/DEBIAN/postrm"
chmod 0755 "$PKG_DIR/DEBIAN/postrm"

# Package file name per packaging/CONTRACT.md:
# iris-{ver}-linux-x86_64.deb
CONTRACT_DEB="iris-${VERSION}-linux-${ARCH}.deb"
CONTRACT_DEB_PATH="$OUT_DIR/$CONTRACT_DEB"

# Also create Debian standard name: iris_{ver}_{arch}.deb
STD_DEB="iris_${VERSION}_${DEB_ARCH}.deb"
STD_DEB_PATH="$OUT_DIR/$STD_DEB"

echo "Building Debian package with dpkg-deb..."
dpkg-deb --build --root-owner-group "$PKG_DIR" "$CONTRACT_DEB_PATH"

# Create symlink/copy for standard Debian naming
if [[ "$CONTRACT_DEB" != "$STD_DEB" ]]; then
  cp "$CONTRACT_DEB_PATH" "$STD_DEB_PATH"
fi

# Generate SHA256 checksum sidecar
if [[ "$GEN_SHA" == true ]]; then
  (cd "$OUT_DIR" && sha256sum "$CONTRACT_DEB" > "$CONTRACT_DEB.sha256")
  echo "Created checksum: $CONTRACT_DEB_PATH.sha256"
fi

echo "Successfully built Debian package:"
echo "  $CONTRACT_DEB_PATH"
if [[ -f "$STD_DEB_PATH" ]]; then
  echo "  $STD_DEB_PATH"
fi
