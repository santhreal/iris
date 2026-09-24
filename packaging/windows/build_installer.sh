#!/usr/bin/env bash
set -euo pipefail

# ==============================================================================
# build_installer.sh — Builds the Windows NSIS setup installer for iris
#
# References:
#   packaging/CONTRACT.md
#
# Release asset name:
#   iris-{ver}-windows-x86_64-setup.exe
#   iris-{ver}-windows-x86_64-setup.exe.sha256
#
# Usage:
#   ./build_installer.sh [options]
#
# Options:
#   -b, --binary <path>    Path to iris.exe executable
#   -o, --out <dir>        Output directory for installer (default: dist)
#   -n, --name <name>      Custom installer filename (default: iris-{ver}-windows-x86_64-setup.exe)
#   -v, --version <ver>    App version (default: from Cargo.toml)
#   -i, --icon <path>      Path to iris.ico (default: packaging/icons/iris.ico)
#   -s, --skip-sha         Skip generating .sha256 checksum sidecar
#   -c, --check            Check/validate NSIS script syntax using a temporary stub binary
#   -h, --help             Show this help message
# ==============================================================================

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

show_help() {
  cat << EOF
Usage: $(basename "$0") [options]

Builds the Windows NSIS installer for iris.

Options:
  -b, --binary <path>    Path to iris.exe binary
  -o, --out <dir>        Output directory (default: <repo_root>/dist)
  -n, --name <name>      Installer executable name
  -v, --version <ver>    Version string (default: read from Cargo.toml)
  -i, --icon <path>      Path to iris.ico (default: packaging/icons/iris.ico)
  -s, --skip-sha         Skip generating .sha256 checksum sidecar
  -c, --check            Syntax-check NSIS script with a temporary stub binary
  -h, --help             Show this help message

Examples:
  $(basename "$0")
  $(basename "$0") --binary target/x86_64-pc-windows-msvc/release/iris.exe
  $(basename "$0") --check
EOF
}

IRIS_BIN=""
OUT_DIR=""
SETUP_NAME=""
VERSION=""
ICON_PATH=""
GEN_SHA=true
SYNTAX_CHECK=false

# Parse command line flags
while [[ $# -gt 0 ]]; do
  case "$1" in
    -b|--binary|--bin)
      IRIS_BIN="$2"
      shift 2
      ;;
    -o|--out)
      OUT_DIR="$2"
      shift 2
      ;;
    -n|--name)
      SETUP_NAME="$2"
      shift 2
      ;;
    -v|--version)
      VERSION="$2"
      shift 2
      ;;
    -i|--icon)
      ICON_PATH="$2"
      shift 2
      ;;
    -s|--skip-sha)
      GEN_SHA=false
      shift
      ;;
    -c|--check|--syntax-only)
      SYNTAX_CHECK=true
      shift
      ;;
    -h|--help)
      show_help
      exit 0
      ;;
    -*)
      echo "Error: Unknown option $1" >&2
      show_help >&2
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

# 1. Verify that makensis is installed
if ! command -v makensis >/dev/null 2>&1; then
  echo "Error: makensis (NSIS compiler) not found on PATH." >&2
  echo "" >&2
  echo "Please install NSIS:" >&2
  echo "  - Ubuntu/Debian:  sudo apt-get install nsis" >&2
  echo "  - Fedora/RHEL:    sudo dnf install mingw32-nsis" >&2
  echo "  - Arch Linux:     sudo pacman -S nsis" >&2
  echo "  - macOS (Homebrew): brew install makensis" >&2
  echo "  - Windows:        winget install NSIS.NSIS  (or choco install nsis)" >&2
  exit 1
fi

# 2. Determine application version
if [[ -z "$VERSION" ]]; then
  if [[ -f "$REPO_ROOT/Cargo.toml" ]]; then
    VERSION=$(grep -m1 '^version = ' "$REPO_ROOT/Cargo.toml" | sed -E 's/version = "([^"]+)"/\1/')
  fi
fi
VERSION="${VERSION:-0.1.0}"

# Format 4-part version for NSIS VIProductVersion (X.X.X.X)
DOT_COUNT=$(echo "$VERSION" | tr -cd '.' | wc -c)
if [[ "$DOT_COUNT" -eq 1 ]]; then
  VERSION_QUAD="${VERSION}.0.0"
elif [[ "$DOT_COUNT" -eq 2 ]]; then
  VERSION_QUAD="${VERSION}.0"
else
  VERSION_QUAD="$VERSION"
fi

# 3. Locate icon file
ICON_PATH="${ICON_PATH:-$REPO_ROOT/packaging/icons/iris.ico}"
if [[ ! -f "$ICON_PATH" ]]; then
  echo "Error: Icon file not found at: $ICON_PATH" >&2
  echo "Please generate icons via: python3 packaging/gen_icons.py" >&2
  exit 1
fi
ICON_PATH="$(cd "$(dirname "$ICON_PATH")" && pwd)/$(basename "$ICON_PATH")"

# 4. Handle syntax-only check or locate iris.exe binary
CLEANUP_STUB=""
if [[ "$SYNTAX_CHECK" == "true" ]]; then
  STUB_DIR="$REPO_ROOT/dist/.nsi_check_$$"
  mkdir -p "$STUB_DIR"
  CLEANUP_STUB="$STUB_DIR"
  trap 'rm -rf "$CLEANUP_STUB"' EXIT
  IRIS_BIN="$STUB_DIR/iris.exe"
  touch "$IRIS_BIN"
else
  if [[ -z "$IRIS_BIN" ]]; then
    SEARCH_PATHS=(
      "$REPO_ROOT/target/x86_64-pc-windows-msvc/release/iris.exe"
      "$REPO_ROOT/target/x86_64-pc-windows-gnu/release/iris.exe"
      "$REPO_ROOT/target/release/iris.exe"
      "$REPO_ROOT/target/x86_64-pc-windows-msvc/debug/iris.exe"
      "$REPO_ROOT/target/x86_64-pc-windows-gnu/debug/iris.exe"
      "$REPO_ROOT/target/debug/iris.exe"
      "$REPO_ROOT/iris.exe"
    )
    for candidate in "${SEARCH_PATHS[@]}"; do
      if [[ -f "$candidate" ]]; then
        IRIS_BIN="$candidate"
        break
      fi
    done
  fi

  if [[ -z "$IRIS_BIN" || ! -f "$IRIS_BIN" ]]; then
    echo "Error: Windows executable (iris.exe) not found." >&2
    if [[ -n "$IRIS_BIN" ]]; then
      echo "  Specified path does not exist: $IRIS_BIN" >&2
    else
      echo "  Searched standard locations:" >&2
      for candidate in "${SEARCH_PATHS[@]}"; do
        echo "    - $candidate" >&2
      done
    fi
    echo "" >&2
    echo "Options to resolve:" >&2
    echo "  1. Build the Windows binary:  cargo build --release --target x86_64-pc-windows-msvc" >&2
    echo "  2. Specify binary path:       $0 --binary <path/to/iris.exe>" >&2
    echo "  3. Validate NSIS syntax only: $0 --check" >&2
    exit 1
  fi
fi

IRIS_BIN="$(cd "$(dirname "$IRIS_BIN")" && pwd)/$(basename "$IRIS_BIN")"

# 5. Output installer filename and target path
# CONTRACT.md specifies: iris-{ver}-windows-x86_64-setup.exe
SETUP_NAME="${SETUP_NAME:-iris-${VERSION}-windows-x86_64-setup.exe}"
if [[ "$SETUP_NAME" != *.exe ]]; then
  SETUP_NAME="${SETUP_NAME}.exe"
fi

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
OUT_FILE="$OUT_DIR/$SETUP_NAME"

echo "==> Building Windows NSIS Installer"
echo "    Version:     $VERSION (quad: $VERSION_QUAD)"
echo "    Binary:      $IRIS_BIN"
echo "    Icon:        $ICON_PATH"
echo "    Output:      $OUT_FILE"

# 6. Execute makensis compiler. Under Git Bash, makensis finds no file
# for a File path in the C:/dir/file form MSYS converts arguments to;
# cygpath gives the C:\dir\file form. Elsewhere paths pass unchanged.
nsis_path() {
  if command -v cygpath >/dev/null 2>&1; then
    cygpath -w "$1"
  else
    printf '%s\n' "$1"
  fi
}

makensis \
  -DVERSION="$VERSION" \
  -DVERSION_QUAD="$VERSION_QUAD" \
  -DBINARY_PATH="$(nsis_path "$IRIS_BIN")" \
  -DICON_PATH="$(nsis_path "$ICON_PATH")" \
  -DOUTFILE="$(nsis_path "$OUT_FILE")" \
  "$(nsis_path "$SCRIPT_DIR/iris.nsi")"

# 7. Generate SHA-256 sidecar (matching CONTRACT.md: each asset + .sha256 sidecar)
if [[ "$GEN_SHA" == "true" && "$SYNTAX_CHECK" == "false" ]]; then
  echo "==> Generating SHA-256 sidecar..."
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$OUT_DIR" && sha256sum "$SETUP_NAME" > "${SETUP_NAME}.sha256")
  elif command -v shasum >/dev/null 2>&1; then
    (cd "$OUT_DIR" && shasum -a 256 "$SETUP_NAME" > "${SETUP_NAME}.sha256")
  else
    echo "Warning: Neither sha256sum nor shasum found; skipping checksum." >&2
  fi
  echo "    Sidecar:     ${OUT_FILE}.sha256"
fi

echo "==> Installer created successfully: $OUT_FILE"
