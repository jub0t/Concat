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

target=${1:?target triple: aarch64-linux-android, x86_64-linux-android or aarch64-apple-ios}
workspace=$(cd "$(dirname "$0")/.." && pwd)
ffmpeg=$workspace/vendor/ffmpeg/$target
[ -d "$ffmpeg/lib" ] || { echo "no FFmpeg for $target: run scripts/ffmpeg-mobile.sh $target" >&2; exit 1; }
echo "export FFMPEG_DIR='$ffmpeg'"

case "$target" in
  aarch64-linux-android|x86_64-linux-android)
    abi=arm64-v8a
    [ "$target" = x86_64-linux-android ] && abi=x86_64
    sherpa=$workspace/vendor/sherpa-onnx/jniLibs/$abi
    [ -d "$sherpa" ] || { echo "no sherpa-onnx for $target: run scripts/sherpa-mobile.sh $target" >&2; exit 1; }
    echo "export SHERPA_ONNX_LIB_DIR='$sherpa'"
    # The emulator's x86_64 build: ort publishes no ONNX Runtime for it,
    # and sherpa-onnx's own (1.27, which serves the API ort asks for) is
    # beside it, so ort links that one, and the APK carries it with the
    # rest of the folder.
    if [ "$target" = x86_64-linux-android ]; then
      echo "export ORT_LIB_LOCATION='$sherpa' ORT_PREFER_DYNAMIC_LINK=1"
    fi

    sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}
    ndk=${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-$(ls -d "$sdk"/ndk/* 2>/dev/null | sort -V | tail -1 || true)}}
    [ -d "$ndk" ] || { echo "no Android NDK: set ANDROID_NDK_HOME" >&2; exit 1; }
    # Every name the toolchain is asked by: cargo-ndk and cargo-apk read
    # ANDROID_NDK_HOME, CMake's Android platform (whisper.cpp) reads
    # ANDROID_NDK_ROOT, and skia-bindings reads ANDROID_NDK.
    echo "export ANDROID_HOME='$sdk' ANDROID_NDK_HOME='$ndk' ANDROID_NDK_ROOT='$ndk' ANDROID_NDK='$ndk'"
    host_tag=$(ls "$ndk/toolchains/llvm/prebuilt" | head -1)
    bindgen_target=$(echo "${target//-/_}" | tr '[:lower:]' '[:upper:]')
    echo "export BINDGEN_EXTRA_CLANG_ARGS_${bindgen_target}='--sysroot=$ndk/toolchains/llvm/prebuilt/$host_tag/sysroot --target=$target'"
    # bindgen needs libclang from the NDK, not the host's clang. The system's
    # libclang (Ubuntu 18.1.3 here) expects glibc headers which don't exist
    # for Android's bionic libc. Search the NDK toolchain tree for libclang.so
    # and point LIBCLANG_PATH at it; also add the NDK's clang bin dir to PATH
    # so bindgen can find the right clang executable.
    libclang_so=$(find "$ndk/toolchains/llvm/prebuilt/$host_tag" -name "libclang.so*" -type f 2>/dev/null | head -1)
    if [ -n "$libclang_so" ]; then
      echo "export LIBCLANG_PATH='$libclang_so'"
      libclang_dir=$(dirname "$libclang_so")
      echo "export LD_LIBRARY_PATH='$libclang_dir:\${LD_LIBRARY_PATH:-}'"
    fi
    # Tell bindgen which clang to use: the NDK's own clang for the target.
    echo "export CLANG_PATH='$ndk/toolchains/llvm/prebuilt/$host_tag/bin/$target$android_api-clang'"
    # Make sure PATH finds the NDK's clang before any host clang.
    echo "export PATH='$ndk/toolchains/llvm/prebuilt/$host_tag/bin:\$PATH'"
    # Cargo target-specific compiler variables so cargo-apk and bindgen use
    # the NDK's clang, not the host's clang which expects glibc headers.
    target_env=$(echo "${target//-/_}" | tr '[:lower:]' '[:upper:]')
    echo "export CC_${target_env}='$ndk/toolchains/llvm/prebuilt/$host_tag/bin/$target$android_api-clang'"
    echo "export CXX_${target_env}='$ndk/toolchains/llvm/prebuilt/$host_tag/bin/$target$android_api-clang++'"
    echo "export AR_${target_env}='$ndk/toolchains/llvm/prebuilt/$host_tag/bin/llvm-ar'"
    # Cargo target-specific compiler variables so cargo-apk and bindgen use
    # the NDK's clang, not the host's clang which expects glibc headers.
    target_env=$(echo "${target//-/_}" | tr '[:lower:]' '[:upper:]')
    echo "export CC_${target_env}='$ndk/toolchains/llvm/prebuilt/$host_tag/bin/$target$android_api-clang'"
    echo "export CXX_${target_env}='$ndk/toolchains/llvm/prebuilt/$host_tag/bin/$target$android_api-clang++'"
    echo "export AR_${target_env}='$ndk/toolchains/llvm/prebuilt/$host_tag/bin/llvm-ar'"
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
    #
    # Every library the APK carries is linked for 16 KB pages, as FFmpeg's
    # already is (ffmpeg-mobile.sh): a phone whose kernel uses them refuses
    # a library aligned to 4 KB, and v0.2.8's libconcat_android.so was.
    # One RUSTFLAGS for both: cargo reads it whole and ignores the
    # target's own flags when it is set.
    rustflags='-C link-arg=-Wl,-z,max-page-size=16384'
    if [ "$(uname -s)" = Darwin ]; then
      shim=$workspace/vendor/shim/$target
      mkdir -p "$shim"
      printf '!<arch>\n' > "$shim/libggml-blas.a"
      rustflags="$rustflags -L native=$shim"
    fi
    echo "export RUSTFLAGS='$rustflags'"
    ;;
  aarch64-apple-ios)
    sherpa=$workspace/vendor/sherpa-onnx/ios/lib
    [ -d "$sherpa" ] || { echo "no sherpa-onnx for $target: run scripts/sherpa-mobile.sh $target" >&2; exit 1; }
    echo "export SHERPA_ONNX_LIB_DIR='$sherpa'"
    echo "export IPHONEOS_DEPLOYMENT_TARGET='${IPHONEOS_DEPLOYMENT_TARGET:-15.0}'"
    ;;
  *)
    echo "unknown target $target" >&2; exit 1 ;;
esac
