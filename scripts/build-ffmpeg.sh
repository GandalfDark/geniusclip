#!/usr/bin/env bash
# Cross-compiles a minimal LGPL FFmpeg for GeniusClip (Windows x64) on Linux.
#
# Why a custom build:
#  * NVENC headers from SDK 12.0 → works with NVIDIA drivers >= 522 (stock
#    FFmpeg 9 builds need >= 610), while still supporting AV1 on RTX 40+.
#  * Only the components GeniusClip uses → ~10x smaller DLLs.
#
# Requires: mingw-w64, nasm, pkg-config, make, git, curl, llvm (llvm-dlltool).
set -euo pipefail

FFMPEG_REF="${FFMPEG_REF:-n9.0.2}"
NVHDR_REF="${NVHDR_REF:-sdk/12.0}"
AMF_REF="${AMF_REF:-v1.5.2}"
ZLIB_VERSION="${ZLIB_VERSION:-1.3.1}"

ROOT="$(pwd)/ffbuild"
SRC="$ROOT/src"
PREFIX="$ROOT/deps"
OUT="$ROOT/out"
HOST=x86_64-w64-mingw32
JOBS="$(nproc)"

rm -rf "$ROOT"
mkdir -p "$SRC" "$PREFIX/include" "$PREFIX/lib" "$OUT"
cd "$SRC"

echo "::group::zlib $ZLIB_VERSION"
curl -fsSL "https://github.com/madler/zlib/releases/download/v$ZLIB_VERSION/zlib-$ZLIB_VERSION.tar.gz" | tar xz
make -C "zlib-$ZLIB_VERSION" -f win32/Makefile.gcc PREFIX="$HOST-" libz.a -j"$JOBS"
cp "zlib-$ZLIB_VERSION"/{zlib.h,zconf.h} "$PREFIX/include/"
cp "zlib-$ZLIB_VERSION/libz.a" "$PREFIX/lib/"
echo "::endgroup::"

echo "::group::nv-codec-headers $NVHDR_REF"
git clone --depth 1 -b "$NVHDR_REF" https://github.com/FFmpeg/nv-codec-headers.git nvhdr
make -C nvhdr PREFIX="$PREFIX" install
echo "::endgroup::"

echo "::group::AMF headers $AMF_REF"
git clone --depth 1 --filter=blob:none --sparse -b "$AMF_REF" https://github.com/GPUOpen-LibrariesAndSDKs/AMF.git amf
git -C amf sparse-checkout set amf/public/include
mkdir -p "$PREFIX/include/AMF"
cp -r amf/amf/public/include/* "$PREFIX/include/AMF/"
echo "::endgroup::"

echo "::group::FFmpeg $FFMPEG_REF"
git clone --depth 1 -b "$FFMPEG_REF" https://github.com/FFmpeg/FFmpeg.git ffmpeg
cd ffmpeg
PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig" ./configure \
    --prefix="$OUT" \
    --arch=x86_64 --target-os=mingw32 --cross-prefix="$HOST-" \
    --pkg-config=pkg-config \
    --extra-cflags="-I$PREFIX/include -O2" \
    --extra-ldflags="-L$PREFIX/lib -static-libgcc" \
    --enable-shared --disable-static \
    --disable-debug --disable-doc --disable-programs --disable-network \
    --disable-autodetect --disable-everything \
    --disable-avdevice --disable-avfilter \
    --enable-w32threads --enable-zlib \
    --enable-d3d11va --enable-mediafoundation \
    --enable-ffnvcodec --enable-nvenc --enable-amf \
    --enable-encoder=h264_nvenc,hevc_nvenc,av1_nvenc,h264_amf,hevc_amf,av1_amf,h264_mf,hevc_mf,av1_mf,aac,png,mjpeg \
    --enable-decoder=h264,hevc,av1,aac,png,mjpeg \
    --enable-hwaccel=h264_d3d11va,hevc_d3d11va,av1_d3d11va \
    --enable-parser=h264,hevc,av1,aac,png,mjpeg \
    --enable-demuxer=mov \
    --enable-muxer=mp4,mov \
    --enable-protocol=file
make -j"$JOBS"
make install
cp COPYING.LGPLv2.1 "$OUT/LICENSE.txt"
cd ..
echo "::endgroup::"

echo "::group::MSVC import libraries"
cd "$OUT/lib"
for def in *.def; do
    base="${def%.def}"          # avcodec-63
    name="${base%-*}"           # avcodec
    llvm-dlltool -m i386:x86-64 -d "$def" -l "$name.lib" -D "$base.dll"
done
ls -la "$OUT/bin" "$OUT/lib"
echo "::endgroup::"

cat > "$OUT/BUILDINFO.txt" <<EOF
FFmpeg:           $FFMPEG_REF
nv-codec-headers: $NVHDR_REF
AMF headers:      $AMF_REF
zlib:             $ZLIB_VERSION
License:          LGPL-2.1-or-later
EOF
