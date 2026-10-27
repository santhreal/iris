#!/usr/bin/env bash
# build_ffmpeg.sh: cross-builds a minimal Windows FFmpeg runtime from
# pinned sources on Linux. x264, libvpx, Opus, and zlib are static.
# gdigrab and DShow capture Windows screens and microphones. NVENC uses
# pinned headers and loads the NVIDIA driver only when selected.
#
#   packaging/windows/build_ffmpeg.sh --arch x86_64|aarch64 --out <dir>
#       [--jobs <n>]
#
# The output holds ffmpeg.exe and licenses/. Run
# packaging/check_ffmpeg.sh on the executable natively on Windows.
set -euo pipefail
umask 022

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
# shellcheck source=packaging/lib.sh
. "$REPO_ROOT/packaging/lib.sh"
# shellcheck source=packaging/ffmpeg_source.sh
. "$REPO_ROOT/packaging/ffmpeg_source.sh"

if [[ $(uname -s) != Linux ]]; then
  echo "Error: cross-building the Windows FFmpeg runtime requires Linux" >&2
  exit 1
fi
ARCH=""
OUT_DIR=""
JOBS=$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)
while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch) ARCH=${2:?--arch needs x86_64 or aarch64}; shift 2 ;;
    --out) OUT_DIR=${2:?--out needs a directory}; shift 2 ;;
    --jobs) JOBS=${2:?--jobs needs a number}; shift 2 ;;
    -h|--help) iris_help "$0"; exit 0 ;;
    *) echo "Error: unknown argument $1 (see --help)" >&2; exit 2 ;;
  esac
done
if [[ -z "$ARCH" || -z "$OUT_DIR" ]]; then
  echo "Error: --arch and --out are required (see --help)" >&2
  exit 2
fi
case "$ARCH" in
  x86_64)
    TRIPLE=x86_64-w64-mingw32
    VPX_TARGET=x86_64-win64-gcc
    ;;
  aarch64)
    TRIPLE=aarch64-w64-mingw32
    VPX_TARGET=arm64-win64-gcc
    ;;
  *) echo "Error: --arch must be x86_64 or aarch64" >&2; exit 2 ;;
esac
for tool in tar make pkg-config patch; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "Error: cross-building FFmpeg requires $tool" >&2
    exit 1
  }
done
if [[ "$ARCH" == x86_64 ]] && ! command -v nasm >/dev/null 2>&1; then
  echo "Error: cross-building x86_64 FFmpeg requires nasm" >&2
  exit 1
fi

TOOLCHAIN_VERSION=20261020
TOOLCHAIN_BASE="llvm-mingw-$TOOLCHAIN_VERSION-ucrt-ubuntu-22.04"
case $(uname -m) in
  x86_64)
    TOOLCHAIN_ARCHIVE="$TOOLCHAIN_BASE-x86_64.tar.xz"
    TOOLCHAIN_SHA=55064b60b991a1e1e8090b8c2f84e82b22cfe5903d12c159d65064cf5e8ae335
    ;;
  aarch64)
    TOOLCHAIN_ARCHIVE="$TOOLCHAIN_BASE-aarch64.tar.xz"
    TOOLCHAIN_SHA=ca16bb0f4f485876496fa46ad8d353c525eba57d0cf9e9e49ec1738fe4810151
    ;;
  *) echo "Error: build on an x86_64 or aarch64 Linux host" >&2; exit 1 ;;
esac
TOOLCHAIN_URL="https://github.com/mstorsjo/llvm-mingw/releases/download/$TOOLCHAIN_VERSION/$TOOLCHAIN_ARCHIVE"
CACHE="$REPO_ROOT/.build-staging/ffmpeg-source"
iris_fetch_sha256 "$TOOLCHAIN_URL" "$TOOLCHAIN_SHA" "$CACHE/$TOOLCHAIN_ARCHIVE"

WORK=$(iris_staging_dir "$REPO_ROOT" ffmpeg-build-windows)
trap 'iris_ffmpeg_source_cleanup $? "$WORK"' EXIT
tar -xf "$CACHE/$TOOLCHAIN_ARCHIVE" -C "$WORK"
TOOLCHAIN="$WORK/${TOOLCHAIN_ARCHIVE%.tar.xz}"
PREFIX="$WORK/prefix"
mkdir -p "$PREFIX"
CC="$TOOLCHAIN/bin/$TRIPLE-clang"
CXX="$TOOLCHAIN/bin/$TRIPLE-clang++"
AR="$TOOLCHAIN/bin/$TRIPLE-ar"
RANLIB="$TOOLCHAIN/bin/$TRIPLE-ranlib"
NM="$TOOLCHAIN/bin/$TRIPLE-nm"
STRIP="$TOOLCHAIN/bin/$TRIPLE-strip"
for tool in "$CC" "$CXX" "$AR" "$RANLIB" "$NM" "$STRIP"; do
  [[ -x "$tool" ]] || { echo "Error: the pinned toolchain lacks $tool" >&2; exit 1; }
done
export CC CXX AR RANLIB NM STRIP
export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig"
export PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig"
unset PKG_CONFIG_SYSROOT_DIR

iris_ffmpeg_source_extract "$REPO_ROOT" "$WORK"
# libvpx has a native Win32 worker implementation, but its MinGW
# configure checks select the pthread DLL when the toolchain provides
# one. Select the existing Win32 implementation in both detection
# stages; the resulting runtime needs no libwinpthread-1.dll.
patch -p1 -d "$WORK/libvpx-1.15.2" <<'PATCH'
--- a/build/make/configure.sh
+++ b/build/make/configure.sh
@@ -1622,7 +1622,7 @@
   # Almost every platform uses pthreads.
   if enabled multithread; then
     case ${toolchain} in
-      *-win*-vs*)
+      *-win*-vs*|*-win*-gcc)
         ;;
       *-android-gcc)
         # bionic includes basic pthread functionality, obviating -lpthread.
--- a/configure
+++ b/configure
@@ -628,12 +628,17 @@
     # check system headers
 
     # Use both check_header and check_lib here, since check_lib
     # could be a stub that always returns true.
-    check_header pthread.h && check_lib -lpthread <<EOF || disable_feature pthread_h
+    case ${toolchain} in
+      *-win*-gcc) disable_feature pthread_h ;;
+      *)
+        check_header pthread.h && check_lib -lpthread <<EOF || disable_feature pthread_h
 #include <pthread.h>
 #include <stddef.h>
 int main(void) { return pthread_create(NULL, NULL, NULL, NULL); }
 EOF
+        ;;
+    esac
     check_header unistd.h # for sysconf(3) and friends.
 
     check_header vpx/vpx_integer.h -I${source_path} && enable_feature vpx_ports
PATCH
iris_ffmpeg_source_nv_headers "$REPO_ROOT" "$WORK" "$PREFIX"
iris_ffmpeg_source_component "$WORK" "$PREFIX" "$JOBS" zlib-1.3.1 \
  --static
iris_ffmpeg_source_component "$WORK" "$PREFIX" "$JOBS" "$X264_DIR" \
  --host="$TRIPLE" --cross-prefix="$TOOLCHAIN/bin/$TRIPLE-" \
  --enable-static --disable-cli --disable-opencl
iris_ffmpeg_source_component "$WORK" "$PREFIX" "$JOBS" libvpx-1.15.2 \
  --target="$VPX_TARGET" --disable-shared --enable-static \
  --disable-examples --disable-tools --disable-docs --disable-unit-tests \
  --enable-vp8 --enable-vp9
OPUS_ARGS=(
  --host="$TRIPLE" --disable-shared --enable-static
  --disable-doc --disable-extra-programs
)
# Opus has no Windows-on-ARM CPU feature query. ARM64 Windows has the
# baseline NEON instructions Opus uses without runtime detection.
[[ "$ARCH" == aarch64 ]] && OPUS_ARGS+=(--disable-rtcd)
iris_ffmpeg_source_component "$WORK" "$PREFIX" "$JOBS" opus-1.6.1 \
  "${OPUS_ARGS[@]}"

iris_ffmpeg_source_component "$WORK" "$PREFIX" "$JOBS" ffmpeg-9.0.2 \
  --arch="$ARCH" --target-os=mingw32 \
  --cross-prefix="$TOOLCHAIN/bin/$TRIPLE-" \
  --cc="$CC" --cxx="$CXX" --ar="$AR" --ranlib="$RANLIB" --nm="$NM" --strip="$STRIP" \
  --pkg-config=pkg-config \
  --pkg-config-flags=--static \
  --disable-autodetect \
  --disable-debug \
  --disable-doc \
  --disable-ffplay \
  --disable-ffprobe \
  --disable-pthreads \
  --enable-w32threads \
  --enable-gpl \
  --enable-zlib \
  --enable-libx264 \
  --enable-libvpx \
  --enable-libopus \
  --enable-ffnvcodec \
  --enable-nvenc \
  --enable-indev=gdigrab \
  --enable-indev=dshow

mkdir -p "$OUT_DIR"
install -m 0755 "$PREFIX/bin/ffmpeg.exe" "$OUT_DIR/ffmpeg.exe"
iris_ffmpeg_source_notices "$WORK" "$OUT_DIR" \
  "Windows $ARCH, static x264/libvpx/Opus/zlib, gdigrab, DShow and NVENC"
install -m 0644 "$WORK/zlib-1.3.1/LICENSE" "$OUT_DIR/licenses/zlib.txt"
iris_ffmpeg_source_nv_notices "$WORK" "$OUT_DIR"
install -m 0644 "$TOOLCHAIN/LICENSE.TXT" "$OUT_DIR/licenses/LLVM.txt"
install -m 0644 \
  "$TOOLCHAIN/$TRIPLE/share/mingw32/COPYING.MinGW-w64-runtime.txt" \
  "$OUT_DIR/licenses/mingw-w64-runtime.txt"
cat >>"$OUT_DIR/licenses/FFMPEG-SOURCE.txt" <<EOF

zlib 1.3.1: $ZLIB_SOURCE_URL
SHA-256: $ZLIB_SOURCE_SHA
Cross toolchain: $TOOLCHAIN_URL
SHA-256: $TOOLCHAIN_SHA
The embedded libvpx configure patch selects its native Win32 worker
implementation instead of the pthread DLL. LLVM and MinGW-w64 runtime
notices cover the statically linked compiler and Windows CRT support.
EOF
BIN=$(iris_ffmpeg_bin "$OUT_DIR" .exe)
iris_resolve_arch "$BIN" "$ARCH" >/dev/null
printf 'Built %s\n' "$BIN"
