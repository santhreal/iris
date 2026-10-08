#!/usr/bin/env bash
# build_deb.sh: builds the Debian package of iris.
#
#   packaging/linux/build_deb.sh --bin <path> --out <dir>
#       [--version <ver>] [--arch x86_64|aarch64] [--skip-sha]
#
#   --bin <path>      the iris executable to package
#   --out <dir>       the directory the package is written to
#   --version <ver>   the iris version (default: Cargo.toml)
#   --arch <arch>     x86_64 or aarch64 (default: the executable's);
#                     fails when the executable is built for another
#   --skip-sha        write no .sha256 sidecar
#
# Writes iris-{ver}-linux-{arch}.deb (packaging/CONTRACT.md) and its
# .sha256 sidecar. The package Version is {ver} with `~` for the
# prerelease separator, and its Architecture is amd64 or arm64.
# Requires dpkg-deb and readelf.
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
DEB_VERSION=$(iris_tilde_version "$VERSION")
ARCH=$(iris_resolve_arch "$IRIS_BIN" "$ARCH")
case "$ARCH" in
  x86_64) DEB_ARCH=amd64 ;;
  aarch64) DEB_ARCH=arm64 ;;
esac

for tool in dpkg-deb readelf; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "Error: building a deb requires $tool (Debian/Ubuntu: apt-get install dpkg binutils)" >&2
    exit 1
  fi
done

# The newest glibc symbol version the binary requires (weak references
# excepted) is the oldest libc6 it runs on.
GLIBC_MIN=$(readelf -V --wide "$IRIS_BIN" \
  | grep -oE 'Name: GLIBC_[0-9.]+ +Flags: none' \
  | grep -oE '[0-9]+\.[0-9.]+' | sort -V | tail -n1 || true)
if [[ -z "$GLIBC_MIN" ]]; then
  echo "Error: no glibc version requirement found in $IRIS_BIN" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"
DEB_NAME="iris-${VERSION}-linux-${ARCH}.deb"
DEB_PATH="$OUT_DIR/$DEB_NAME"
echo "Packaging $IRIS_BIN as $DEB_PATH (Version $DEB_VERSION, Architecture $DEB_ARCH)"

TMP_DIR=$(iris_staging_dir "$REPO_ROOT" deb)
trap 'rm -rf "$TMP_DIR"' EXIT
PKG_DIR="$TMP_DIR/pkg"
DOC_DIR="$PKG_DIR/usr/share/doc/iris"
mkdir -p "$PKG_DIR/DEBIAN" "$PKG_DIR/usr/bin" "$PKG_DIR/usr/share/applications" \
  "$PKG_DIR/usr/share/metainfo" "$PKG_DIR/etc/xdg/autostart" "$DOC_DIR"

install -m 0755 "$IRIS_BIN" "$PKG_DIR/usr/bin/iris"
install -m 0644 "$SCRIPT_DIR/dev.iris.app.desktop" "$PKG_DIR/usr/share/applications/"
install -m 0644 "$SCRIPT_DIR/iris-autostart.desktop" "$PKG_DIR/etc/xdg/autostart/"
install -m 0644 "$SCRIPT_DIR/dev.iris.app.metainfo.xml" "$PKG_DIR/usr/share/metainfo/"

iris_hicolor_icons "$REPO_ROOT" "$PKG_DIR/usr/share/icons/hicolor"

# debian/copyright in the machine-readable format 1.0, with the text
# of every license it names.
license_text() {
  sed -e 's/[[:space:]]*$//' -e 's/^$/./' -e 's/^/ /' "$1"
}
{
  cat <<'EOF'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: iris
Upstream-Contact: Santh <64453045+santhreal@users.noreply.github.com>
Source: https://github.com/santhreal/iris

Files: *
Copyright: 2026 Santh <64453045+santhreal@users.noreply.github.com>
License: MIT or Apache-2.0

Files: assets/fonts/Inter-*
Copyright: 2016 The Inter Project Authors (https://github.com/rsms/inter)
License: OFL-1.1

License: MIT
EOF
  license_text "$REPO_ROOT/LICENSE-MIT"
  echo
  echo "License: Apache-2.0"
  license_text "$REPO_ROOT/LICENSE-APACHE"
  echo
  echo "License: OFL-1.1"
  license_text "$REPO_ROOT/assets/fonts/Inter-OFL.txt"
} >"$DOC_DIR/copyright"
chmod 0644 "$DOC_DIR/copyright"

INSTALLED_SIZE=$(du -sk "$PKG_DIR" | cut -f1)
cat >"$PKG_DIR/DEBIAN/control" <<EOF
Package: iris
Version: ${DEB_VERSION}
Section: utils
Priority: optional
Architecture: ${DEB_ARCH}
Maintainer: Santh <64453045+santhreal@users.noreply.github.com>
Installed-Size: ${INSTALLED_SIZE}
Homepage: https://github.com/santhreal/iris
Depends: libc6 (>= ${GLIBC_MIN}), libpipewire-0.3-0 (>= 0.3.0), libxkbcommon0, libxkbcommon-x11-0, libxcb1, libfontconfig1, libwayland-client0, libvulkan1, libegl1, libgles2
Recommends: ffmpeg, pkexec | policykit-1
Suggests: tesseract-ocr
Description: Screenshot and screen-recording utility
 iris is a screenshot and screen-recording tool for X11 and Wayland. A
 background daemon holds the global hotkeys and the tray icon. A capture
 freezes the screen for region selection, saves a PNG, copies it to the
 clipboard, and shows a thumbnail toast that opens the annotation editor.
 Recording runs ffmpeg; the text copy in the toast and the editor runs
 tesseract.
EOF
chmod 0644 "$PKG_DIR/DEBIAN/control"
install -m 0755 "$SCRIPT_DIR/deb/postinst" "$SCRIPT_DIR/deb/postrm" "$PKG_DIR/DEBIAN/"

dpkg-deb --build --root-owner-group "$PKG_DIR" "$DEB_PATH"
if [[ "$GEN_SHA" == true ]]; then
  iris_sha256_sidecar "$DEB_PATH"
fi
echo "Built $DEB_PATH"
