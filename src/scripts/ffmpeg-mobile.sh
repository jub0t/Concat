#!/usr/bin/env bash
# Builds FFmpeg's libraries for a phone, from source, as static archives
# the engine links.
#
#   scripts/ffmpeg-mobile.sh aarch64-linux-android [out-dir]
#   scripts/ffmpeg-mobile.sh x86_64-linux-android [out-dir]   (the emulator's, never shipped)
#
# The result is a prefix - include/ and lib/ - that concat-media's
# bindings take through FFMPEG_DIR, exactly as they take a Homebrew or
# BtbN build on the desktop. It lands in vendor/ffmpeg/<target> under the
# engine by default, outside target/, so a `cargo clean` does not cost
# another FFmpeg build and CI's cache can keep it.
#
# It needs the NDK (ANDROID_NDK_HOME, or the newest one under the SDK in
# ANDROID_HOME / ~/Library/Android/sdk). The build is LGPL FFmpeg with the
# phone's hardware codecs turned on - MediaCodec, through the JNI - and
# nothing else linked in: a phone encodes with its silicon, and the GPL
# encoders the desktop bundles have no place there.
set -euo pipefail

target=${1:?target triple: aarch64-linux-android or x86_64-linux-android}
workspace=$(cd "$(dirname "$0")/.." && pwd)
out=${2:-$workspace/vendor/ffmpeg/$target}
version=${FFMPEG_VERSION:-8.1}
# The oldest Android the build runs on: 8.0 is where AAudio, the audio
# path the engine plays through, appears.
android_api=${ANDROID_API:-26}
jobs=$(getconf _NPROCESSORS_ONLN 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)

work=${FFMPEG_WORK_DIR:-$workspace/vendor/ffmpeg/src}
src=$work/ffmpeg-$version
mkdir -p "$work"
if [ ! -f "$src/configure" ]; then
  echo "Fetching FFmpeg $version"
  curl -fsSL --retry 5 --retry-all-errors -o "$work/ffmpeg-$version.tar.xz" \
    "https://ffmpeg.org/releases/ffmpeg-$version.tar.xz"
  tar -xf "$work/ffmpeg-$version.tar.xz" -C "$work"
fi

# One build tree per target, so two targets can be built from one source.
build=$work/build-$target
rm -rf "$build"
mkdir -p "$build"

# Flags shared by every phone build. No programs, no docs, no shared
# libraries: the engine links the archives into one binary. Position
# independent because Android loads the app as a shared object.
common=(
  --prefix="$out"
  --enable-cross-compile
  --enable-static --disable-shared --enable-pic
  --disable-programs --disable-doc
  --disable-debug
  # Nothing external. zlib is part of the NDK; bzip2 and lzma are not,
  # so neither is taken.
  --enable-zlib --disable-bzlib --disable-lzma
  --disable-iconv --disable-sdl2 --disable-xlib --disable-libxcb
  --disable-vulkan --disable-opencl --disable-vaapi --disable-vdpau
)

case "$target" in
  aarch64-linux-android|x86_64-linux-android)
    ndk=${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}
    if [ -z "$ndk" ]; then
      sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}
      ndk=$(ls -d "$sdk"/ndk/* 2>/dev/null | sort -V | tail -1 || true)
    fi
    [ -d "$ndk" ] || { echo "no Android NDK: set ANDROID_NDK_HOME" >&2; exit 1; }
    host_tag=$(ls "$ndk/toolchains/llvm/prebuilt" | head -1)
    bin=$ndk/toolchains/llvm/prebuilt/$host_tag/bin
    echo "Building FFmpeg $version for $target with NDK at $ndk (API $android_api)"
    # The x86_64 build is the one CI's emulators can run, never shipped:
    # FFmpeg's hand-written assembly is left out there, so the build needs
    # no assembler, and its speed is nobody's concern.
    if [ "$target" = aarch64-linux-android ]; then
      arch=(--arch=aarch64 --cpu=armv8-a)
    else
      arch=(--arch=x86_64 --disable-x86asm)
    fi
    (cd "$build" && "$src/configure" "${common[@]}" \
      --target-os=android "${arch[@]}" \
      --sysroot="$ndk/toolchains/llvm/prebuilt/$host_tag/sysroot" \
      --cc="$bin/$target$android_api-clang" \
      --cxx="$bin/$target$android_api-clang++" \
      --ar="$bin/llvm-ar" --nm="$bin/llvm-nm" --ranlib="$bin/llvm-ranlib" --strip="$bin/llvm-strip" \
      --enable-jni --enable-mediacodec \
      --extra-cflags="-fno-omit-frame-pointer" \
      --extra-ldflags="-Wl,-z,max-page-size=16384")
    ;;
  *)
    echo "unknown target $target" >&2; exit 1 ;;
esac

make -C "$build" -j"$jobs" install
echo "FFmpeg $version for $target is in $out"
echo "  export FFMPEG_DIR=$out"
