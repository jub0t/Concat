#!/usr/bin/env bash
# Fetches the sherpa-onnx runtime libraries for a phone: the shared
# libraries k2-fsa publishes, which the app carries with it.
#
#   scripts/sherpa-mobile.sh aarch64-linux-android [out-dir]
#   scripts/sherpa-mobile.sh x86_64-linux-android [out-dir]   (the emulator's, never shipped)
#   scripts/sherpa-mobile.sh aarch64-apple-ios     [out-dir]
#
# The version is the one the lockfile resolved for sherpa-onnx-sys, so a
# bump of the crate cannot desynchronise from the libraries. The archives
# are rows in scripts/ci/deps.lock and are fetched through
# scripts/ci/fetch.sh, which refuses a file whose digest is not the one
# written there: what ships inside the APK and the IPA is bytes somebody
# has looked at once, as on the desktop.
#
# Android: vendor/sherpa-onnx/jniLibs/<abi>/*.so, the layout an APK's
# native library directory takes, with the NDK's libc++_shared.so beside
# them because onnxruntime loads it. Point SHERPA_ONNX_LIB_DIR at the ABI
# directory to build, and cargo-apk packages the whole tree through
# concat-android's manifest.
#
# iOS: vendor/sherpa-onnx/ios/sherpa-onnx.xcframework, the framework the
# app bundle embeds; SHERPA_ONNX_LIB_DIR is its device slice.
set -euo pipefail

target=${1:?target triple: aarch64-linux-android, x86_64-linux-android or aarch64-apple-ios}
workspace=$(cd "$(dirname "$0")/.." && pwd)
version=$(grep -A1 'name = "sherpa-onnx-sys"' "$workspace/Cargo.lock" | sed -n 's/^version = "\(.*\)"/\1/p')
# The digest-verified fetcher the desktop builds download through, at the
# repository's root: this script lives under src/, scripts/ci above it.
fetch="$workspace/../scripts/ci/fetch.sh"

case "$target" in
  aarch64-linux-android|x86_64-linux-android)
    out=${2:-$workspace/vendor/sherpa-onnx}
    # The ABI's folder, as Android names it.
    abi=arm64-v8a
    [ "$target" = x86_64-linux-android ] && abi=x86_64
    ndk=${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}
    if [ -z "$ndk" ]; then
      sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}
      ndk=$(ls -d "$sdk"/ndk/* 2>/dev/null | sort -V | tail -1 || true)
    fi
    [ -d "$ndk" ] || { echo "no Android NDK: set ANDROID_NDK_HOME" >&2; exit 1; }
    host_tag=$(ls "$ndk/toolchains/llvm/prebuilt" | head -1)
    echo "Fetching sherpa-onnx $version for Android"
    mkdir -p "$out"
    # One archive for every ABI, its digest pinned in scripts/ci/deps.lock;
    # the row must carry the lockfile's version, as ci.yml checks the
    # desktop rows do.
    name=sherpa-onnx-android
    case "$("$fetch" --url "$name")" in
      */v"$version"/*) ;;
      *) echo "scripts/ci/deps.lock pins $name for another version than Cargo.lock's $version" >&2; exit 1 ;;
    esac
    "$fetch" "$name" "$out/sherpa-onnx-android.tar.bz2"
    tar -xjf "$out/sherpa-onnx-android.tar.bz2" -C "$out"
    rm "$out/sherpa-onnx-android.tar.bz2"
    # Only the ABI the app is built for ships; the rest is left behind.
    find "$out/jniLibs" -mindepth 1 -maxdepth 1 -type d ! -name "$abi" -exec rm -rf {} +
    # The C API and its runtime are what the engine loads; the JNI and
    # C++ bindings are for other languages.
    rm -f "$out/jniLibs/$abi/libsherpa-onnx-jni.so" "$out/jniLibs/$abi/libsherpa-onnx-cxx-api.so"
    cp "$ndk/toolchains/llvm/prebuilt/$host_tag/sysroot/usr/lib/$target/libc++_shared.so" \
      "$out/jniLibs/$abi/"
    echo "sherpa-onnx for Android is in $out/jniLibs"
    echo "  export SHERPA_ONNX_LIB_DIR=$out/jniLibs/$abi"
    ;;
  aarch64-apple-ios)
    out=${2:-$workspace/vendor/sherpa-onnx/ios}
    echo "Fetching sherpa-onnx $version for iOS"
    mkdir -p "$out"
    # The xcframework zip, its digest pinned in scripts/ci/deps.lock as the
    # Android archive's is. The iOS archives hang off k2-fsa's
    # `xcframework` tag rather than a versioned one, so the version the row
    # must carry is in the file's own name.
    name=sherpa-onnx-ios-xcframework
    case "$("$fetch" --url "$name")" in
      *sherpa-onnx-v"$version"-*) ;;
      *) echo "scripts/ci/deps.lock pins $name for another version than Cargo.lock's $version" >&2; exit 1 ;;
    esac
    "$fetch" "$name" "$out/sherpa-onnx-ios.zip"
    (cd "$out" && unzip -qo sherpa-onnx-ios.zip && rm sherpa-onnx-ios.zip)
    framework=$(find "$out" -maxdepth 2 -name '*.xcframework' | head -1)
    [ -n "$framework" ] || { echo "no xcframework in the archive" >&2; exit 1; }
    binary=$framework/ios-arm64/SherpaOnnxC.framework/SherpaOnnxC
    [ -f "$binary" ] || { echo "no device slice in $framework" >&2; exit 1; }
    # The linker looks for the library by its lib-prefixed name; the
    # framework carries the dylib under the framework's own.
    mkdir -p "$out/lib"
    ln -sf "$binary" "$out/lib/libsherpa-onnx-c-api.dylib"
    echo "sherpa-onnx for iOS is $framework"
    echo "  export SHERPA_ONNX_LIB_DIR=$out/lib"
    ;;
  *)
    echo "unknown target $target" >&2; exit 1 ;;
esac
