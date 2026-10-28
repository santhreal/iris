# shellcheck shell=bash
# Shared source pins and static dependencies for the macOS and Linux
# FFmpeg runtimes. Source after packaging/lib.sh.

FFMPEG_SOURCE_URL=https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz
FFMPEG_SOURCE_SHA=8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e
X264_SOURCE_URL=https://github.com/mirror/x264/archive/0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee.tar.gz
X264_SOURCE_SHA=d0967a1348c85dfde363bb52610403be898171493100561efa0dd05d5fd1ae50
VPX_SOURCE_URL=https://github.com/webmproject/libvpx/archive/refs/tags/v1.15.2.tar.gz
VPX_SOURCE_SHA=26fcd3db88045dee380e581862a6ef106f49b74b6396ee95c2993a260b4636aa
OPUS_SOURCE_URL=https://downloads.xiph.org/releases/opus/opus-1.6.1.tar.gz
OPUS_SOURCE_SHA=6ffcb593207be92584df15b32466ed64bbec99109f007c82205f0194572411a1
ZLIB_SOURCE_URL=https://github.com/madler/zlib/releases/download/v1.3.1/zlib-1.3.1.tar.gz
ZLIB_SOURCE_SHA=9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23
NV_HEADERS_URL=https://github.com/FFmpeg/nv-codec-headers/archive/refs/tags/n13.1.15.0.tar.gz
NV_HEADERS_SHA=2255bc74d038b95aa4be30f5f66322c2176acbdb90ada1851db6993536fbeaf7
NV_HEADERS_DIR=nv-codec-headers-n13.1.15.0

X264_DIR=x264-0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee

# iris_ffmpeg_source_cleanup STATUS WORK
# Removes a successful build tree and retains a failed one for diagnosis.
iris_ffmpeg_source_cleanup() {
  local status=$1 work=$2
  if [[ "$status" -eq 0 ]]; then
    rm -rf "$work"
  else
    echo "FFmpeg build files retained at $work" >&2
  fi
}

# iris_ffmpeg_source_extract REPO_ROOT WORK
# Downloads every pinned source archive and extracts it into WORK.
iris_ffmpeg_source_extract() {
  local root=$1 work=$2 cache="$1/.build-staging/ffmpeg-source"
  local spec file url sha archive
  for spec in \
    "ffmpeg-9.0.2.tar.xz|$FFMPEG_SOURCE_URL|$FFMPEG_SOURCE_SHA" \
    "x264-0480cb05.tar.gz|$X264_SOURCE_URL|$X264_SOURCE_SHA" \
    "libvpx-1.15.2.tar.gz|$VPX_SOURCE_URL|$VPX_SOURCE_SHA" \
    "opus-1.6.1.tar.gz|$OPUS_SOURCE_URL|$OPUS_SOURCE_SHA" \
    "zlib-1.3.1.tar.gz|$ZLIB_SOURCE_URL|$ZLIB_SOURCE_SHA"; do
    IFS='|' read -r file url sha <<<"$spec"
    iris_fetch_sha256 "$url" "$sha" "$cache/$file"
  done
  for archive in ffmpeg-9.0.2.tar.xz x264-0480cb05.tar.gz libvpx-1.15.2.tar.gz opus-1.6.1.tar.gz zlib-1.3.1.tar.gz; do
    tar -xf "$cache/$archive" -C "$work"
  done
}

# iris_ffmpeg_source_component WORK PREFIX JOBS DIR [configure args...]
iris_ffmpeg_source_component() (
  local work=$1 prefix=$2 jobs=$3 dir=$4
  shift 4
  echo "Configuring $dir"
  cd "$work/$dir" || exit
  ./configure --prefix="$prefix" "$@" && make -j"$jobs" && make install
)

# iris_ffmpeg_source_build WORK PREFIX JOBS
# Builds the static libraries shared by the Linux and macOS runtimes.
iris_ffmpeg_source_build() {
  local work=$1 prefix=$2 jobs=$3
  iris_ffmpeg_source_component "$work" "$prefix" "$jobs" "$X264_DIR" \
    --enable-static --enable-pic --disable-cli --disable-opencl
  iris_ffmpeg_source_component "$work" "$prefix" "$jobs" libvpx-1.15.2 \
    --disable-shared --enable-static --enable-pic --disable-examples --disable-tools \
    --disable-docs --disable-unit-tests --enable-vp8 --enable-vp9
  iris_ffmpeg_source_component "$work" "$prefix" "$jobs" opus-1.6.1 \
    --disable-shared --enable-static --with-pic --disable-doc --disable-extra-programs
}

# iris_ffmpeg_source_nv_headers REPO_ROOT WORK PREFIX
# Installs the pinned NVENC headers. FFmpeg loads the NVIDIA driver only
# when a recording selects NVENC.
iris_ffmpeg_source_nv_headers() {
  local root=$1 work=$2 prefix=$3
  local archive="$root/.build-staging/ffmpeg-source/nv-codec-headers-13.1.15.0.tar.gz"
  iris_fetch_sha256 "$NV_HEADERS_URL" "$NV_HEADERS_SHA" "$archive"
  tar -xf "$archive" -C "$work"
  make -C "$work/$NV_HEADERS_DIR" install PREFIX="$prefix"
}

# iris_ffmpeg_source_nv_notices WORK OUT_DIR
# Adds the NVENC header license and corresponding-source location.
iris_ffmpeg_source_nv_notices() {
  local work=$1 out=$2
  sed -n '1,/^ \*\/$/p' "$work/$NV_HEADERS_DIR/include/ffnvcodec/nvEncodeAPI.h" \
    >"$out/licenses/nv-codec-headers.txt"
  cat >>"$out/licenses/FFMPEG-SOURCE.txt" <<EOF

NVENC headers 13.1.15.0: $NV_HEADERS_URL
SHA-256: $NV_HEADERS_SHA
The NVIDIA driver is loaded by FFmpeg only when NVENC is selected.
EOF
}

# iris_ffmpeg_compiler_rt_notices REPO_ROOT OUT_DIR
# Adds the license and contributor list for Clang's static builtins.
# Call after iris_ffmpeg_source_notices, which creates FFMPEG-SOURCE.txt.
iris_ffmpeg_compiler_rt_notices() {
  local root=$1 out=$2
  install -m 0644 "$root/packaging/licenses/compiler-rt.txt" \
    "$out/licenses/compiler-rt.txt"
  install -m 0644 "$root/packaging/licenses/compiler-rt-CREDITS.txt" \
    "$out/licenses/compiler-rt-CREDITS.txt"
  cat >>"$out/licenses/FFMPEG-SOURCE.txt" <<'EOF'

Compiler-rt license and credits:
  https://github.com/llvm/llvm-project/tree/llvmorg-23.1.4/compiler-rt
License SHA-256: 1a8f1058753f1ba890de984e48f0242a3a5c29a6a8f2ed9fd813f36985387e8d
Credits SHA-256: a9901f47a089da41e4690682d00ce4cedaa2baf41fedbe79beee366d43ac2461
The notice bytes are identical in LLVM 15.0.0, 17.0.0, and 23.1.4.
EOF
}

# iris_ffmpeg_source_notices WORK OUT_DIR BUILD_DESCRIPTION
# Installs notices shared by all source runtimes and records their common
# sources. Callers add the notices of platform-specific static libraries.
iris_ffmpeg_source_notices() {
  local work=$1 out=$2 description=$3
  mkdir -p "$out/licenses"
  install -m 0644 "$work/ffmpeg-9.0.2/COPYING.GPLv2" "$out/licenses/FFmpeg.txt"
  install -m 0644 "$work/ffmpeg-9.0.2/LICENSE.md" "$out/licenses/FFmpeg-LICENSE.txt"
  install -m 0644 "$work/$X264_DIR/COPYING" "$out/licenses/x264.txt"
  install -m 0644 "$work/libvpx-1.15.2/LICENSE" "$out/licenses/libvpx.txt"
  install -m 0644 "$work/opus-1.6.1/COPYING" "$out/licenses/Opus.txt"
  cat >"$out/licenses/FFMPEG-SOURCE.txt" <<EOF
FFmpeg corresponding source and build provenance

Build: $description
FFmpeg 9.0.2: $FFMPEG_SOURCE_URL
SHA-256: $FFMPEG_SOURCE_SHA
x264 commit 0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee:
  Upstream: https://code.videolan.org/videolan/x264/-/commit/0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee
  Archive: $X264_SOURCE_URL
  SHA-256: $X264_SOURCE_SHA
libvpx 1.15.2: $VPX_SOURCE_URL
SHA-256: $VPX_SOURCE_SHA
Opus 1.6.1: $OPUS_SOURCE_URL
SHA-256: $OPUS_SOURCE_SHA

The build script and iris packaging source are at
https://github.com/santhreal/iris . The configuration contains static
libraries, no nonfree components, and the platform capture support
named by the build description above.
EOF
}
