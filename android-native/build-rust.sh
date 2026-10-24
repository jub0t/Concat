#!/usr/bin/env bash
# Builds the engine for the spike shell and generates its Kotlin bindings:
#
#   android-native/build-rust.sh [release|debug]
#
# libconcat_mobile.so goes into app/src/main/jniLibs/arm64-v8a beside the
# runtime libraries the engine links (libc++_shared, ONNX Runtime), and
# the UniFFI Kotlin into app/src/main/java/uniffi.
#
# FFmpeg and sherpa-onnx come from the main checkout's vendor/ (built by
# src/scripts/ffmpeg-mobile.sh and sherpa-mobile.sh) when this checkout
# has none, and CARGO_TARGET_DIR defaults to the main checkout's target/,
# so a worktree never starts a second one.
set -euo pipefail

profile=${1:-release}
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/.." && pwd)
workspace=$repo/src
vendor_from=$workspace
if [ ! -d "$workspace/vendor/ffmpeg/aarch64-linux-android" ]; then
  # A worktree: the main checkout is the one whose .git is a directory.
  main=$(git -C "$repo" worktree list --porcelain | sed -n '1s/^worktree //p')
  vendor_from=$main/src
fi
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$vendor_from/target}

eval "$("$vendor_from/scripts/mobile-env.sh" aarch64-linux-android)"

flags=()
[ "$profile" = release ] && flags+=(--release)
cd "$workspace"
started=$(date +%s)
cargo ndk -t arm64-v8a --platform 26 build "${flags[@]}" -p concat-mobile
echo "cargo ndk took $(( $(date +%s) - started ))s"

so=$CARGO_TARGET_DIR/aarch64-linux-android/$profile/libconcat_mobile.so
libs=$here/app/src/main/jniLibs/arm64-v8a
mkdir -p "$libs"
cp "$so" "$libs/"
# The runtime libraries the Slint APK carries too, where the engine needs
# them: ask the linker's own record.
needed=$("$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*/bin/llvm-readelf -d "$so" | sed -n 's/.*Shared library: \[\(.*\)\]/\1/p')
echo "libconcat_mobile.so needs: $(echo $needed)"
for lib in $needed; do
  if [ -f "$SHERPA_ONNX_LIB_DIR/$lib" ]; then cp "$SHERPA_ONNX_LIB_DIR/$lib" "$libs/"; fi
done
if echo "$needed" | grep -q 'libc++_shared.so' && [ ! -f "$libs/libc++_shared.so" ]; then
  cp "$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so "$libs/"
fi

# The bindings, from the library's own metadata, by a bindgen built for
# this machine from the same uniffi.
(
  unset RUSTFLAGS
  cd "$here/uniffi-bindgen"
  cargo run -q --release -- \
    generate --library "$so" --language kotlin --no-format \
    --out-dir "$here/app/src/main/java"
)
ls -la "$libs"
