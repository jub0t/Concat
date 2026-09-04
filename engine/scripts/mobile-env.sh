#!/usr/bin/env bash
# The build environment for a phone, as shell exports:
#
#   eval "$(scripts/mobile-env.sh aarch64-linux-android)"
#   cargo ndk -t arm64-v8a --platform 26 build -p concat-android
#
#   eval "$(scripts/mobile-env.sh aarch64-apple-ios)"
#   cargo build -p concat --target aarch64-apple-ios
#
# It names what the two fetch scripts left in vendor/ - FFmpeg through
# FFMPEG_DIR, sherpa-onnx through SHERPA_ONNX_LIB_DIR - and on Android the
# SDK pieces the build scripts of Slint's backend and whisper.cpp read.
# Run scripts/ffmpeg-mobile.sh and scripts/sherpa-mobile.sh for the target
# first.
set -euo pipefail

target=${1:?target triple: aarch64-linux-android or aarch64-apple-ios}
engine=$(cd "$(dirname "$0")/.." && pwd)
ffmpeg=$engine/vendor/ffmpeg/$target
[ -d "$ffmpeg/lib" ] || { echo "no FFmpeg for $target: run scripts/ffmpeg-mobile.sh $target" >&2; exit 1; }
echo "export FFMPEG_DIR='$ffmpeg'"

case "$target" in
  aarch64-linux-android)
    sherpa=$engine/vendor/sherpa-onnx/jniLibs/arm64-v8a
    [ -d "$sherpa" ] || { echo "no sherpa-onnx for $target: run scripts/sherpa-mobile.sh $target" >&2; exit 1; }
    echo "export SHERPA_ONNX_LIB_DIR='$sherpa'"

    sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}
    ndk=${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-$(ls -d "$sdk"/ndk/* 2>/dev/null | sort -V | tail -1 || true)}}
    [ -d "$ndk" ] || { echo "no Android NDK: set ANDROID_NDK_HOME" >&2; exit 1; }
    # Every name the toolchain is asked by: cargo-ndk and cargo-apk read
    # ANDROID_NDK_HOME, CMake's Android platform (whisper.cpp) reads
    # ANDROID_NDK_ROOT, and skia-bindings reads ANDROID_NDK.
    echo "export ANDROID_HOME='$sdk' ANDROID_NDK_HOME='$ndk' ANDROID_NDK_ROOT='$ndk' ANDROID_NDK='$ndk'"
    # bindgen reads FFmpeg's headers with its own libclang, which knows
    # nothing of the NDK until told where the sysroot is.
    host_tag=$(ls "$ndk/toolchains/llvm/prebuilt" | head -1)
    echo "export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android='--sysroot=$ndk/toolchains/llvm/prebuilt/$host_tag/sysroot'"
    # Slint's backend compiles a Java helper against the SDK platform's
    # android.jar. cargo-ndk names the NDK API level as ANDROID_PLATFORM,
    # which is also the name the lookup reads for the SDK platform, and the
    # two need not be installed together; the jar is named outright.
    platform=$(ls -d "$sdk"/platforms/android-* 2>/dev/null | sort -V | tail -1 || true)
    [ -f "$platform/android.jar" ] || { echo "no SDK platform under $sdk/platforms" >&2; exit 1; }
    echo "export ANDROID_JAR='$platform/android.jar'"

    # whisper-rs-sys names ggml-blas on the link line whenever the machine
    # doing the build is a Mac, a fact it reads with cfg!(target_os) in its
    # build script, where that is the host and not the target. Building
    # for Android on a Mac, ggml has no BLAS and no such archive. An empty
    # archive under that name, on the search path, is what the linker finds
    # instead, and it contributes exactly what the missing library would
    # have. The ar magic alone is a well-formed archive with no members.
    if [ "$(uname -s)" = Darwin ]; then
      shim=$engine/vendor/shim/$target
      mkdir -p "$shim"
      printf '!<arch>\n' > "$shim/libggml-blas.a"
      echo "export RUSTFLAGS='-L native=$shim'"
    fi
    ;;
  aarch64-apple-ios)
    sherpa=$engine/vendor/sherpa-onnx/ios/lib
    [ -d "$sherpa" ] || { echo "no sherpa-onnx for $target: run scripts/sherpa-mobile.sh $target" >&2; exit 1; }
    echo "export SHERPA_ONNX_LIB_DIR='$sherpa'"
    echo "export IPHONEOS_DEPLOYMENT_TARGET='${IPHONEOS_DEPLOYMENT_TARGET:-15.0}'"
    ;;
  *)
    echo "unknown target $target" >&2; exit 1 ;;
esac
