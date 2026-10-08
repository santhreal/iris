#!/usr/bin/env bash
# build_rpm.sh: builds the RPM package of iris with rpmbuild and
# packaging/linux/iris.spec.
#
#   packaging/linux/build_rpm.sh --bin <path> --out <dir>
#       [--version <ver>] [--arch x86_64|aarch64] [--skip-sha]
#
#   --bin <path>      the iris executable to package
#   --out <dir>       the directory the package is written to
#   --version <ver>   the iris version (default: Cargo.toml)
#   --arch <arch>     x86_64 or aarch64 (default: the executable's);
#                     fails when the executable is built for another
#   --skip-sha        write no .sha256 sidecar
#
# Writes iris-{ver}-linux-{arch}.rpm (packaging/CONTRACT.md) and its
# .sha256 sidecar. The package Version is {ver} with `~` for the
# prerelease separator, and its Arch is x86_64 or aarch64. Requires
# rpmbuild (Debian/Ubuntu: rpm, Fedora: rpm-build).
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
RPM_VERSION=$(iris_tilde_version "$VERSION")
ARCH=$(iris_resolve_arch "$IRIS_BIN" "$ARCH")

if ! command -v rpmbuild >/dev/null 2>&1; then
  echo "Error: building an rpm requires rpmbuild (Debian/Ubuntu: apt-get install rpm, Fedora: dnf install rpm-build)" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"
RPM_NAME="iris-${VERSION}-linux-${ARCH}.rpm"
RPM_PATH="$OUT_DIR/$RPM_NAME"
echo "Packaging $IRIS_BIN as $RPM_PATH (Version $RPM_VERSION, Arch $ARCH)"

TOPDIR=$(iris_staging_dir "$REPO_ROOT" rpm)
trap 'rm -rf "$TOPDIR"' EXIT
mkdir -p "$TOPDIR"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}

# Source0 holds the prebuilt binary and the files the spec installs,
# at their paths in the repository; %autosetup unpacks it.
SOURCE_DIR="$TOPDIR/src/iris-${RPM_VERSION}"
mkdir -p "$SOURCE_DIR/packaging/linux" "$SOURCE_DIR/packaging/icons" "$SOURCE_DIR/assets/fonts"
install -m 0755 "$IRIS_BIN" "$SOURCE_DIR/iris"
cp "$SCRIPT_DIR/dev.iris.app.desktop" "$SCRIPT_DIR/iris-autostart.desktop" \
  "$SCRIPT_DIR/dev.iris.app.metainfo.xml" "$SOURCE_DIR/packaging/linux/"
cp "$REPO_ROOT"/packaging/icons/iris-*.png "$SOURCE_DIR/packaging/icons/"
cp "$REPO_ROOT/LICENSE-MIT" "$REPO_ROOT/LICENSE-APACHE" "$SOURCE_DIR/"
cp "$REPO_ROOT/assets/fonts/Inter-OFL.txt" "$SOURCE_DIR/assets/fonts/"
tar -czf "$TOPDIR/SOURCES/iris-${RPM_VERSION}.tar.gz" -C "$TOPDIR/src" "iris-${RPM_VERSION}"

sed "s/^Version: .*/Version:        ${RPM_VERSION}/" "$SCRIPT_DIR/iris.spec" >"$TOPDIR/SPECS/iris.spec"

# --nodeps: the BuildRequires list the toolchain of a build from
# source; this package holds a binary built beforehand.
rpmbuild -bb --nodeps \
  --define "_topdir $TOPDIR" \
  --target "$ARCH" \
  "$TOPDIR/SPECS/iris.spec"

BUILT=("$TOPDIR/RPMS/$ARCH"/iris-*."$ARCH".rpm)
if [[ ${#BUILT[@]} -ne 1 || ! -f "${BUILT[0]}" ]]; then
  echo "Error: rpmbuild wrote no single $ARCH rpm in $TOPDIR/RPMS/$ARCH" >&2
  exit 1
fi
cp "${BUILT[0]}" "$RPM_PATH"
if [[ "$GEN_SHA" == true ]]; then
  iris_sha256_sidecar "$RPM_PATH"
fi
echo "Built $RPM_PATH"
