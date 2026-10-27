#!/usr/bin/env bash
# check_ffmpeg.sh: verifies an FFmpeg runtime on its target platform.
#
#   packaging/check_ffmpeg.sh <ffmpeg path> <linux|macos|windows>
#
# The runtime must be redistributable and contain the encoders Iris
# uses. Linux and Windows include NVENC and their microphone inputs;
# macOS includes AVFoundation. Real Matroska, MP4, WebM, and GIF files
# are encoded and decoded in a temporary directory and checked for
# nonzero size.
set -euo pipefail
umask 022

if [[ $# -ne 2 || ! -f $1 ]]; then
  printf 'Usage: %s <ffmpeg path> <linux|macos|windows>\n' "$0" >&2
  exit 2
fi
BIN=$1
PLATFORM=$2
case "$PLATFORM" in
  linux|macos|windows) ;;
  *) echo "Error: platform must be linux, macos, or windows" >&2; exit 2 ;;
esac

VERSION=$("$BIN" -version 2>&1)
if [[ "$VERSION" == *--enable-nonfree* ]]; then
  echo "Error: $BIN enables nonfree components and cannot be redistributed" >&2
  exit 1
fi
if [[ "$VERSION" != *--enable-gpl* ]]; then
  echo "Error: $BIN lacks the GPL configuration required for x264" >&2
  exit 1
fi
printf '%s\n' "${VERSION%%$'\n'*}"

ENCODERS=$("$BIN" -hide_banner -encoders 2>/dev/null)
DEVICES=$("$BIN" -hide_banner -devices 2>/dev/null)
require_component() {
  local kind=$1 name=$2 text=$3
  local pattern="[[:space:]]${name}([[:space:]]|$)"
  if [[ "$kind" == input ]]; then
    pattern="^[[:space:]]*D[E[:space:]][[:space:]]+${name}([[:space:],]|$)"
  fi
  if ! grep -Eq "$pattern" <<<"$text"; then
    echo "Error: $BIN lacks required $kind $name" >&2
    exit 1
  fi
}
for encoder in libx264 libx264rgb libvpx-vp9 libopus aac; do
  require_component encoder "$encoder" "$ENCODERS"
done
case "$PLATFORM" in
  linux)
    require_component encoder h264_nvenc "$ENCODERS"
    require_component input pulse "$DEVICES"
    ;;
  windows)
    require_component encoder h264_nvenc "$ENCODERS"
    require_component input gdigrab "$DEVICES"
    require_component input dshow "$DEVICES"
    ;;
  macos)
    require_component input avfoundation "$DEVICES"
    ;;
esac

mkdir -p "$PWD/.build-staging"
WORK=$(mktemp -d "$PWD/.build-staging/ffmpeg-check.XXXXXX")
trap 'rm -rf "$WORK"' EXIT
native_path() {
  if [[ "$PLATFORM" == windows ]] && command -v cygpath >/dev/null 2>&1; then
    cygpath -w "$1"
  else
    printf '%s\n' "$1"
  fi
}
SEGMENT=$(native_path "$WORK/segment.mkv")
MP4=$(native_path "$WORK/out.mp4")
WEBM=$(native_path "$WORK/out.webm")
PALETTE=$(native_path "$WORK/palette.png")
GIF=$(native_path "$WORK/out.gif")

"$BIN" -hide_banner -loglevel error -y \
  -f lavfi -i color=black:s=256x256:d=0.2:r=5 \
  -c:v libx264rgb -preset ultrafast -qp 0 -bf 0 -f matroska "$SEGMENT"
"$BIN" -hide_banner -loglevel error -y \
  -i "$SEGMENT" -f lavfi -i sine=frequency=1000:duration=0.2 \
  -map 0:v -map 1:a -c:v libx264 -preset ultrafast -pix_fmt yuv420p \
  -c:a aac -b:a 128k -ar 48000 -ac 2 -shortest -movflags +faststart "$MP4"
"$BIN" -hide_banner -loglevel error -y \
  -i "$SEGMENT" -f lavfi -i sine=frequency=1000:duration=0.2 \
  -map 0:v -map 1:a -c:v libvpx-vp9 -deadline realtime -cpu-used 8 \
  -row-mt 1 -b:v 0 -crf 32 -pix_fmt yuv420p \
  -c:a libopus -b:a 96k -ar 48000 -ac 2 -shortest "$WEBM"
"$BIN" -hide_banner -loglevel error -y -i "$SEGMENT" \
  -vf palettegen -frames:v 1 -update 1 "$PALETTE"
"$BIN" -hide_banner -loglevel error -y -i "$SEGMENT" -i "$PALETTE" \
  -lavfi '[0:v][1:v]paletteuse=diff_mode=rectangle' -frames:v 1 "$GIF"
for file in "$WORK/segment.mkv" "$WORK/out.mp4" "$WORK/out.webm" "$WORK/palette.png" "$WORK/out.gif"; do
  [[ -s "$file" ]] || { echo "Error: $BIN wrote no $file" >&2; exit 1; }
done
for file in "$WORK/segment.mkv" "$WORK/out.mp4" "$WORK/out.webm" "$WORK/out.gif"; do
  "$BIN" -hide_banner -loglevel error -i "$(native_path "$file")" -f null -
done
printf 'FFmpeg runtime passed %s encoder and container checks\n' "$PLATFORM"
