#!/usr/bin/env bash
# Builds FFmpeg's shared libraries for a Mac, from source, LGPL.
#
#   scripts/ffmpeg-macos.sh [out-dir]
#
# The result is a prefix - include/ and lib/ - that concat-media's bindings
# take through FFMPEG_DIR, and whose dylibs the release bundles into the
# app. It replaces Homebrew's FFmpeg, which is GPL and carries x264 and
# x265: Concat encodes H.264 and HEVC on VideoToolbox alone, AAC on
# AudioToolbox, and links nothing GPL. Built for the machine's own
# architecture, natively - an Apple silicon runner for arm64, an Intel one
# for x86_64.
#
# The royalty-free codecs come from Homebrew, each under a BSD licence:
# libvpx (VP9 with alpha), SVT-AV1 (AV1), Opus, and dav1d (AV1 decoding).
#   brew install libvpx svt-av1 opus dav1d pkgconf nasm
# nasm is the x86_64 build's assembler; arm64 needs none.
set -euo pipefail

workspace=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-$workspace/vendor/ffmpeg/macos}
version=${FFMPEG_VERSION:-8.1}
# The oldest macOS the app runs on (#342).
macos_min=${MACOSX_DEPLOYMENT_TARGET:-12.0}
jobs=$(sysctl -n hw.ncpu 2>/dev/null || echo 4)

work=${FFMPEG_WORK_DIR:-$workspace/vendor/ffmpeg/src}
src=$work/ffmpeg-$version
mkdir -p "$work"
if [ ! -f "$src/configure" ]; then
  echo "Fetching FFmpeg $version"
  curl -fsSL --retry 5 --retry-all-errors -o "$work/ffmpeg-$version.tar.xz" \
    "https://ffmpeg.org/releases/ffmpeg-$version.tar.xz"
  tar -xf "$work/ffmpeg-$version.tar.xz" -C "$work"
fi

build=$work/build-macos-$(uname -m)
rm -rf "$build"
mkdir -p "$build"
cd "$build"

echo "Building FFmpeg $version for macOS $(uname -m), from macOS $macos_min, into $out"
"$src/configure" \
  --prefix="$out" \
  --enable-shared --disable-static \
  --disable-programs --disable-doc --disable-debug \
  --disable-avdevice \
  --extra-cflags="-mmacosx-version-min=$macos_min" \
  --extra-ldflags="-mmacosx-version-min=$macos_min" \
  `# LGPL: no --enable-gpl and no --enable-nonfree, so nothing GPL can be` \
  `# linked in even if the machine has it.` \
  --enable-videotoolbox --enable-audiotoolbox \
  --enable-libvpx --enable-libsvtav1 --enable-libopus --enable-libdav1d \
  `# The SDK's own; nothing else from the machine.` \
  --enable-zlib --disable-bzlib --disable-lzma --disable-iconv \
  --disable-sdl2 --disable-xlib --disable-libxcb --disable-securetransport \
  --disable-avfoundation --disable-coreimage --disable-appkit \
  --disable-vulkan --disable-opencl

make -j"$jobs"
make install

# Licensed as FFmpeg says it is, or not shipped.
licence=$(grep -o '#define FFMPEG_LICENSE "[^"]*"' "$build/config.h" | cut -d'"' -f2)
echo "FFmpeg licence: $licence"
case "$licence" in
  LGPL*) ;;
  *) echo "FFmpeg was configured $licence, not LGPL" >&2; exit 1 ;;
esac
