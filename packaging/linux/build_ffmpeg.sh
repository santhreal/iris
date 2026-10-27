#!/usr/bin/env bash
# build_ffmpeg.sh: builds the Linux FFmpeg runtime from pinned sources.
# x264, libvpx, and Opus are static. PulseAudio and zlib use the system
# libraries declared by the iris packages. NVENC uses FFmpeg's pinned
# headers and loads the NVIDIA driver only when a recording selects it.
#
#   packaging/linux/build_ffmpeg.sh --out <dir> [--jobs <n>]
#
# The output holds ffmpeg and licenses/. Build on the target x86_64 or
# aarch64 host, then run packaging/check_ffmpeg.sh on the executable.
set -euo pipefail
umask 022

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
# shellcheck source=packaging/lib.sh
. "$REPO_ROOT/packaging/lib.sh"
# shellcheck source=packaging/ffmpeg_source.sh
. "$REPO_ROOT/packaging/ffmpeg_source.sh"

if [[ $(uname -s) != Linux ]]; then
  echo "Error: the Linux FFmpeg runtime must build on Linux" >&2
  exit 1
fi
case $(uname -m) in
  x86_64|aarch64) ;;
  *) echo "Error: build on x86_64 or aarch64, not $(uname -m)" >&2; exit 1 ;;
esac
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
for tool in tar make pkg-config cc nasm; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "Error: building FFmpeg requires $tool (Debian/Ubuntu: build-essential nasm pkg-config)" >&2
    exit 1
  }
done
for library in libpulse zlib; do
  pkg-config --exists "$library" || {
    echo "Error: building Linux FFmpeg requires $library development files (Debian/Ubuntu: libpulse-dev zlib1g-dev)" >&2
    exit 1
  }
done

WORK=$(iris_staging_dir "$REPO_ROOT" ffmpeg-build)
trap 'iris_ffmpeg_source_cleanup $? "$WORK"' EXIT
PREFIX="$WORK/prefix"
mkdir -p "$PREFIX"
iris_ffmpeg_source_extract "$REPO_ROOT" "$WORK"
iris_ffmpeg_source_nv_headers "$REPO_ROOT" "$WORK" "$PREFIX"

iris_ffmpeg_source_build "$WORK" "$PREFIX" "$JOBS"

export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig"
iris_ffmpeg_source_component "$WORK" "$PREFIX" "$JOBS" ffmpeg-9.0.2 \
  --extra-libs=-lm \
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
  --enable-libpulse \
  --enable-ffnvcodec \
  --enable-nvenc

mkdir -p "$OUT_DIR"
install -m 0755 "$PREFIX/bin/ffmpeg" "$OUT_DIR/ffmpeg"
iris_ffmpeg_source_notices "$WORK" "$OUT_DIR" \
  "Linux $(uname -m), static x264/libvpx/Opus, PulseAudio and NVENC"
iris_ffmpeg_source_nv_notices "$WORK" "$OUT_DIR"
cat >>"$OUT_DIR/licenses/FFMPEG-SOURCE.txt" <<EOF

PulseAudio and zlib are dynamically linked system libraries.
EOF
iris_ffmpeg_bin "$OUT_DIR" >/dev/null
printf 'Built %s\n' "$OUT_DIR/ffmpeg"
