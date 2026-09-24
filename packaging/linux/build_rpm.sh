#!/usr/bin/env bash
set -euo pipefail

# build_rpm.sh — Builds an RPM package for iris using rpmbuild
#
# References:
#   packaging/CONTRACT.md
#
# Release asset name:
#   iris-{ver}-linux-x86_64.rpm
#
# Usage:
#   ./build_rpm.sh [options] [binary-path] [output-dir]
#
# Options:
#   -b, --bin <path>      Path to iris executable binary
#   -o, --out <dir>       Output directory for .rpm (default: dist)
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

# Check for rpmbuild
if ! command -v rpmbuild >/dev/null 2>&1; then
  echo "Error: 'rpmbuild' is required to build RPM packages but was not found." >&2
  echo "On Fedora/RHEL: sudo dnf install rpm-build" >&2
  echo "On Debian/Ubuntu: sudo apt-get install rpm" >&2
  exit 1
fi

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
echo "Packaging iris RPM version: $VERSION ($ARCH)"

# Staging directory inside the repository (.build-staging is gitignored).
STAGING_BASE="$REPO_ROOT/.build-staging"
mkdir -p "$STAGING_BASE"
RPM_TOPDIR="$(mktemp -d "$STAGING_BASE/rpm_build.XXXXXX")"
trap 'rm -rf "$RPM_TOPDIR"' EXIT

mkdir -p "$RPM_TOPDIR"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}

# Source0 is a tarball of the prebuilt binary plus the packaging files,
# unpacked by the spec's %autosetup.
SOURCE_DIR="$RPM_TOPDIR/src/iris-${VERSION}"
mkdir -p "$SOURCE_DIR/packaging/linux"
mkdir -p "$SOURCE_DIR/packaging/icons"

cp "$IRIS_BIN" "$SOURCE_DIR/iris"
chmod 0755 "$SOURCE_DIR/iris"
if command -v strip >/dev/null 2>&1; then
  strip --strip-unneeded "$SOURCE_DIR/iris" 2>/dev/null || true
fi

cp "$SCRIPT_DIR/iris.desktop" "$SOURCE_DIR/packaging/linux/iris.desktop"
cp "$SCRIPT_DIR/iris-autostart.desktop" "$SOURCE_DIR/packaging/linux/iris-autostart.desktop"
if [[ -f "$SCRIPT_DIR/dev.iris.app.metainfo.xml" ]]; then
  cp "$SCRIPT_DIR/dev.iris.app.metainfo.xml" "$SOURCE_DIR/packaging/linux/dev.iris.app.metainfo.xml"
fi

# Copy icons
cp -r "$REPO_ROOT/packaging/icons"/* "$SOURCE_DIR/packaging/icons/"

tar -czf "$RPM_TOPDIR/SOURCES/iris-${VERSION}.tar.gz" -C "$RPM_TOPDIR/src" "iris-${VERSION}"

# Copy spec file and update version if needed
SPEC_FILE="$RPM_TOPDIR/SPECS/iris.spec"
sed "s/^Version: .*/Version:        ${VERSION}/" "$SCRIPT_DIR/iris.spec" > "$SPEC_FILE"

# Run rpmbuild. --nodeps: the BuildRequires list the toolchain for a
# build from source; this package holds a binary built beforehand.
echo "Running rpmbuild..."
rpmbuild -bb \
  --nodeps \
  --define "_topdir $RPM_TOPDIR" \
  --define "_builddir $RPM_TOPDIR/BUILD" \
  --define "_rpmdir $RPM_TOPDIR/RPMS" \
  --define "_sourcedir $RPM_TOPDIR/SOURCES" \
  --define "_specdir $RPM_TOPDIR/SPECS" \
  --define "_srcrpmdir $RPM_TOPDIR/SRPMS" \
  --target "${ARCH}" \
  "$SPEC_FILE"

# Locate generated RPM file
FOUND_RPM=$(find "$RPM_TOPDIR/RPMS" -name "*.rpm" -type f | head -n 1)
if [[ -z "$FOUND_RPM" || ! -f "$FOUND_RPM" ]]; then
  echo "Error: rpmbuild completed but no .rpm file was found." >&2
  exit 1
fi

# Contract RPM filename: iris-{ver}-linux-x86_64.rpm
CONTRACT_RPM="iris-${VERSION}-linux-${ARCH}.rpm"
CONTRACT_RPM_PATH="$OUT_DIR/$CONTRACT_RPM"

cp "$FOUND_RPM" "$CONTRACT_RPM_PATH"

# Generate SHA256 checksum sidecar
if [[ "$GEN_SHA" == true ]]; then
  (cd "$OUT_DIR" && sha256sum "$CONTRACT_RPM" > "$CONTRACT_RPM.sha256")
  echo "Created checksum: $CONTRACT_RPM_PATH.sha256"
fi

echo "Successfully built RPM package:"
echo "  $CONTRACT_RPM_PATH"
