#!/usr/bin/env bash
# build_ffmpeg.sh: builds the static macOS FFmpeg runtime from pinned
# source archives. The binary contains x264, libvpx-vp9, Opus, the
# built-in AAC encoder, and AVFoundation capture. It has no nonfree
# components and no Homebrew or MacPorts runtime dependency.
#
#   packaging/macos/build_ffmpeg.sh --out <dir> [--jobs <n>]
#
# The output holds ffmpeg and licenses/. Run it natively on x86_64 and
# arm64, then combine the two executables with lipo. The package
# workflow does this and runs check_ffmpeg.sh before making the bundle.
set -euo pipefail
umask 022

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
# shellcheck source=packaging/lib.sh
. "$REPO_ROOT/packaging/lib.sh"
# shellcheck source=packaging/ffmpeg_source.sh
. "$REPO_ROOT/packaging/ffmpeg_source.sh"

if [[ $(uname -s) != Darwin ]]; then
  echo "Error: the macOS FFmpeg runtime must build on macOS" >&2
  exit 1
fi
OUT_DIR=""
JOBS=$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)
while [[ $# -gt 0 ]]; do
  case "$1" in
    --out) OUT_DIR=${2:?--out needs a directory}; shift 2 ;;
    --jobs) JOBS=${2:?--jobs needs a number}; shift 2 ;;
    -h|--help) iris_help "$0"; exit 0 ;;
    *) echo "Error: unknown argument $1 (see --help)" >&2; exit 2 ;;
  esac
done
if [[ -z "$OUT_DIR" ]]; then
  echo "Error: --out is required (see --help)" >&2
  exit 2
fi
for tool in tar make pkg-config clang nasm; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "Error: building FFmpeg requires $tool (brew install nasm pkg-config)" >&2
    exit 1
  }
done
case $(uname -m) in
  x86_64) export MACOSX_DEPLOYMENT_TARGET=10.15 ;;
  arm64) export MACOSX_DEPLOYMENT_TARGET=11.0 ;;
  *) echo "Error: build on an x86_64 or arm64 Mac, not $(uname -m)" >&2; exit 1 ;;
esac

WORK=$(iris_staging_dir "$REPO_ROOT" ffmpeg-build)
trap 'iris_ffmpeg_source_cleanup $? "$WORK"' EXIT
PREFIX="$WORK/prefix"
mkdir -p "$PREFIX"
iris_ffmpeg_source_extract "$REPO_ROOT" "$WORK"
iris_ffmpeg_source_build "$WORK" "$PREFIX" "$JOBS"

export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig"
iris_ffmpeg_source_component "$WORK" "$PREFIX" "$JOBS" ffmpeg-9.0.2 \
  --pkg-config-flags=--static \
  --disable-autodetect \
  --disable-debug \
  --disable-doc \
  --disable-ffplay \
  --disable-ffprobe \
  --enable-gpl \
  --enable-iconv \
  --enable-zlib \
  --enable-libx264 \
  --enable-libvpx \
  --enable-libopus \
  --enable-indev=avfoundation

mkdir -p "$OUT_DIR"
install -m 0755 "$PREFIX/bin/ffmpeg" "$OUT_DIR/ffmpeg"
iris_ffmpeg_source_notices "$WORK" "$OUT_DIR" \
  "macOS, static x264/libvpx/Opus, AVFoundation capture"
iris_ffmpeg_bin "$OUT_DIR" >/dev/null
printf 'Built %s\n' "$OUT_DIR/ffmpeg"
