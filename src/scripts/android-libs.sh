#!/usr/bin/env bash
# The shared libraries an Android build carries beside the engine, laid out
# as an APK's native library directory takes them:
# vendor/android/jniLibs/<abi>/*.so.
#
#   scripts/android-libs.sh aarch64-linux-android [out-dir]
#   scripts/android-libs.sh x86_64-linux-android  [out-dir]   (the emulator's, never shipped)
#
# - libc++_shared.so from the NDK, on both: ONNX Runtime is C++, and
#   ort-sys links it against c++_shared on Android.
# - libonnxruntime.so on x86_64 alone. ort downloads a static ONNX Runtime
#   for arm64 phones but publishes none for the emulator's ABI, so there
#   ort links Microsoft's own, from its onnxruntime-android AAR (1.28.0,
#   the version ort is built against), pinned by digest in
#   scripts/ci/deps.lock. mobile-env.sh points ORT_LIB_LOCATION at it.
#
# cargo-apk packages the whole tree through concat-android's manifest.
set -euo pipefail

target=${1:?target triple: aarch64-linux-android or x86_64-linux-android}
workspace=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$workspace/.." && pwd)
out=${2:-$workspace/vendor/android/jniLibs}

case "$target" in
  aarch64-linux-android) abi=arm64-v8a ;;
  x86_64-linux-android) abi=x86_64 ;;
  *) echo "unknown target $target" >&2; exit 1 ;;
esac

ndk=${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}
if [ -z "$ndk" ]; then
  sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}
  ndk=$(ls -d "$sdk"/ndk/* 2>/dev/null | sort -V | tail -1 || true)
fi
[ -d "$ndk" ] || { echo "no Android NDK: set ANDROID_NDK_HOME" >&2; exit 1; }
host_tag=$(ls "$ndk/toolchains/llvm/prebuilt" | head -1)

# Only the ABI the app is built for ships.
mkdir -p "$out/$abi"
find "$out" -mindepth 1 -maxdepth 1 -type d ! -name "$abi" -exec rm -rf {} +

cp "$ndk/toolchains/llvm/prebuilt/$host_tag/sysroot/usr/lib/$target/libc++_shared.so" \
  "$out/$abi/"

if [ "$abi" = x86_64 ]; then
  scratch=$(mktemp -d)
  trap 'rm -rf "$scratch"' EXIT
  "$root/scripts/ci/fetch.sh" onnxruntime-android "$scratch/onnxruntime-android.aar"
  unzip -qo -j "$scratch/onnxruntime-android.aar" "jni/$abi/libonnxruntime.so" -d "$out/$abi"
  [ -f "$out/$abi/libonnxruntime.so" ] || { echo "no $abi ONNX Runtime in the AAR" >&2; exit 1; }
fi

echo "The Android libraries for $target are in $out/$abi"
