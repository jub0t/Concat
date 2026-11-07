#!/usr/bin/env bash
# Stages the ONNX Runtime an Android build links, for the one target the
# `ort` crate has no prebuilt of its own for.
#
#   scripts/android-libs.sh <rust-target-triple>
#
# ort's build script downloads a prebuilt ONNX Runtime for the target it
# is compiling - except that pyke's table, which every target's prebuilt
# comes from, has no x86_64-linux-android. That is exactly the APK CI's
# emulators start (mobile.yml): an x86_64 build of the same source, so the
# launch check runs natively instead of through ARM translation, where
# ONNX Runtime's CPU detection crashes as the library loads. So on that
# target the runtime is Microsoft's own Android build, the libonnxruntime.so
# from the AAR it publishes to Maven Central, unpacked under src/vendor/
# for the target and linked through ORT_LIB_LOCATION with dynamic linking
# preferred.
#
# cargo-apk carries that .so into the APK on its own: it walks the shared
# libraries the activity links against, resolving each against the link
# search paths the build scripts leave behind (ORT_LIB_LOCATION among
# them), and for libc++_shared.so the NDK's own sysroot, which is why no
# C++ runtime is staged here.
#
# The arm64 APK needs none of this - ort's prebuilt covers
# aarch64-linux-android, and the shipping APK links ONNX Runtime
# statically as it always has - so on that target this script does
# nothing.
#
# In CI the two variables go to $GITHUB_ENV, for the build step after
# this one; run by hand it prints the exports to `eval` instead.
set -euo pipefail

target=${1:?usage: android-libs.sh <rust-target-triple>}
repo=$(cd "$(dirname "$0")/.." && pwd)

# The AAR Microsoft publishes for Android, one per ABI. 1.28 is the
# version ort-sys's own prebuilts are built from, and the one whose API
# version is the 27 concat-vision's manifest asks for.
ort_version=1.28.0
ort_sha256=f351a0638696f54b35184290dbc001d66daae17281ad0b548d2c70347d53b8a9

case "$target" in
  x86_64-linux-android)
    abi=x86_64
    ;;
  aarch64-linux-android)
    # Covered by ort-sys's own prebuilt table; nothing to stage.
    exit 0
    ;;
  *)
    echo "android-libs.sh: no ONNX Runtime to stage for $target" >&2
    exit 1
    ;;
esac

dest=$repo/src/vendor/onnxruntime/$target
so=$dest/libonnxruntime.so
if [ ! -f "$so" ]; then
  mkdir -p "$dest"
  aar=$dest/onnxruntime-android-$ort_version.aar
  if [ ! -f "$aar" ]; then
    url=https://repo1.maven.org/maven2/com/microsoft/onnxruntime/onnxruntime-android/$ort_version/onnxruntime-android-$ort_version.aar
    echo "android-libs.sh: fetching ONNX Runtime $ort_version for $abi" >&2
    curl -fL --retry 3 -o "$aar" "$url"
  fi
  echo "$ort_sha256  $aar" | sha256sum -c -
  # An AAR is a zip; the shared library sits under jni/, one directory per
  # ABI, beside a Java binding this build has no use for.
  unzip -p "$aar" "jni/$abi/libonnxruntime.so" > "$so.tmp"
  mv "$so.tmp" "$so"
fi

if [ -n "${GITHUB_ENV:-}" ]; then
  {
    echo "ORT_LIB_LOCATION=$dest"
    echo "ORT_PREFER_DYNAMIC_LINK=1"
  } >> "$GITHUB_ENV"
else
  echo "export ORT_LIB_LOCATION='$dest'"
  echo "export ORT_PREFER_DYNAMIC_LINK=1"
fi
