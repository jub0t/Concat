#!/usr/bin/env bash
# Builds FFmpeg's libraries for a phone, from source, as static archives
# the engine links.
#
#   scripts/ffmpeg-mobile.sh aarch64-linux-android [out-dir]
#   scripts/ffmpeg-mobile.sh aarch64-apple-ios     [out-dir]
#   scripts/ffmpeg-mobile.sh aarch64-apple-ios-sim [out-dir]
#
# The result is a prefix - include/ and lib/ - that concat-media's
# bindings take through FFMPEG_DIR, exactly as they take a Homebrew or
# BtbN build on the desktop. It lands in vendor/ffmpeg/<target> under the
# engine by default, outside target/, so a `cargo clean` does not cost
# another FFmpeg build and CI's cache can keep it.
#
# Android needs the NDK (ANDROID_NDK_HOME, or the newest one under the SDK
# in ANDROID_HOME / ~/Library/Android/sdk); iOS needs Xcode. Both builds
# are LGPL FFmpeg with the platform's hardware codecs turned on -
# MediaCodec through the JNI on Android, VideoToolbox on iOS - and nothing
# else linked in: a phone encodes with its silicon, and the GPL encoders
# the desktop bundles have no place there.
set -euo pipefail

target=${1:?target triple: aarch64-linux-android, aarch64-apple-ios or aarch64-apple-ios-sim}
engine=$(cd "$(dirname "$0")/.." && pwd)
out=${2:-$engine/vendor/ffmpeg/$target}
version=${FFMPEG_VERSION:-8.1}
# Oldest OS each build runs on. Android 8.0 is where AAudio, the audio
# path the engine plays through, appears; iOS 15 is where Slint draws.
android_api=${ANDROID_API:-26}
ios_min=${IPHONEOS_DEPLOYMENT_TARGET:-15.0}
jobs=$(getconf _NPROCESSORS_ONLN 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)

work=${FFMPEG_WORK_DIR:-$engine/vendor/ffmpeg/src}
src=$work/ffmpeg-$version
mkdir -p "$work"
if [ ! -f "$src/configure" ]; then
  echo "Fetching FFmpeg $version"
  curl -fsSL --retry 5 --retry-all-errors -o "$work/ffmpeg-$version.tar.xz" \
    "https://ffmpeg.org/releases/ffmpeg-$version.tar.xz"
  tar -xf "$work/ffmpeg-$version.tar.xz" -C "$work"
fi

# One build tree per target, so two phones can be built from one source.
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
  # Nothing external. zlib is part of both platforms' SDKs.
  --enable-zlib
  --disable-iconv --disable-sdl2 --disable-xlib --disable-libxcb
  --disable-vulkan --disable-opencl --disable-vaapi --disable-vdpau
)

case "$target" in
  aarch64-linux-android)
    ndk=${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}
    if [ -z "$ndk" ]; then
      sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}
      ndk=$(ls -d "$sdk"/ndk/* 2>/dev/null | sort -V | tail -1 || true)
    fi
    [ -d "$ndk" ] || { echo "no Android NDK: set ANDROID_NDK_HOME" >&2; exit 1; }
    host_tag=$(ls "$ndk/toolchains/llvm/prebuilt" | head -1)
    bin=$ndk/toolchains/llvm/prebuilt/$host_tag/bin
    echo "Building FFmpeg $version for $target with NDK at $ndk (API $android_api)"
    (cd "$build" && "$src/configure" "${common[@]}" \
      --target-os=android --arch=aarch64 --cpu=armv8-a \
      --sysroot="$ndk/toolchains/llvm/prebuilt/$host_tag/sysroot" \
      --cc="$bin/aarch64-linux-android$android_api-clang" \
      --cxx="$bin/aarch64-linux-android$android_api-clang++" \
      --ar="$bin/llvm-ar" --nm="$bin/llvm-nm" --ranlib="$bin/llvm-ranlib" --strip="$bin/llvm-strip" \
      --enable-jni --enable-mediacodec \
      --extra-cflags="-fno-omit-frame-pointer" \
      --extra-ldflags="-Wl,-z,max-page-size=16384")
    ;;
  aarch64-apple-ios|aarch64-apple-ios-sim)
    if [ "$target" = aarch64-apple-ios ]; then
      sdk=iphoneos; min_flag="-miphoneos-version-min=$ios_min"
    else
      sdk=iphonesimulator; min_flag="-mios-simulator-version-min=$ios_min"
    fi
    sysroot=$(xcrun --sdk $sdk --show-sdk-path)
    echo "Building FFmpeg $version for $target against $sysroot"
    (cd "$build" && "$src/configure" "${common[@]}" \
      --target-os=darwin --arch=arm64 \
      --sysroot="$sysroot" \
      --cc="$(xcrun --sdk $sdk -f clang)" \
      --cxx="$(xcrun --sdk $sdk -f clang++)" \
      --ar="$(xcrun --sdk $sdk -f ar)" --nm="$(xcrun --sdk $sdk -f nm)" \
      --ranlib="$(xcrun --sdk $sdk -f ranlib)" --strip="$(xcrun --sdk $sdk -f strip)" \
      --enable-videotoolbox --enable-audiotoolbox \
      --extra-cflags="-arch arm64 $min_flag" \
      --extra-ldflags="-arch arm64 $min_flag")
    ;;
  *)
    echo "unknown target $target" >&2; exit 1 ;;
esac

make -C "$build" -j"$jobs" install
echo "FFmpeg $version for $target is in $out"
echo "  export FFMPEG_DIR=$out"
