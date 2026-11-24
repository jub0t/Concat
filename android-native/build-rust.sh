#!/usr/bin/env bash
# Builds the engine for the Android app and generates its Kotlin bindings:
#
#   android-native/build-rust.sh [release|debug] [aarch64-linux-android|x86_64-linux-android]
#
# libconcat_mobile.so goes into app/src/main/jniLibs/<abi> beside the
# runtime libraries the engine links (libc++_shared, ONNX Runtime), and
# the UniFFI Kotlin into app/src/main/java/uniffi.
#
# FFmpeg and the Android libraries come from the main checkout's vendor/
# (made by src/scripts/ffmpeg-mobile.sh and android-libs.sh) when this checkout
# has none, and CARGO_TARGET_DIR defaults to the main checkout's target/,
# so a worktree never starts a second one.
set -euo pipefail

profile=${1:-release}
# The phones' arm64 unless told otherwise; x86_64 is for the emulators CI
# smoke-tests on.
target=${2:-aarch64-linux-android}
case $target in
  aarch64-linux-android) abi=arm64-v8a ;;
  x86_64-linux-android) abi=x86_64 ;;
  *) echo "build-rust.sh: no Android ABI for $target" >&2; exit 2 ;;
esac
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/.." && pwd)
workspace=$repo/src
vendor_from=$workspace
if [ ! -d "$workspace/vendor/ffmpeg/$target" ]; then
  # A worktree: the main checkout is the one whose .git is a directory.
  main=$(git -C "$repo" worktree list --porcelain | sed -n '1s/^worktree //p')
  vendor_from=$main/src
fi
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$vendor_from/target}

eval "$("$vendor_from/scripts/mobile-env.sh" "$target")"

flags=()
[ "$profile" = release ] && flags+=(--release)
cd "$workspace"
started=$(date +%s)
cargo ndk -t "$abi" --platform 26 build "${flags[@]}" -p concat-mobile
echo "cargo ndk took $(( $(date +%s) - started ))s"

so=$CARGO_TARGET_DIR/$target/$profile/libconcat_mobile.so
libs=$here/app/src/main/jniLibs/$abi
mkdir -p "$libs"
cp "$so" "$libs/"
# The runtime libraries the Slint APK carries too, where the engine needs
# them: ask the linker's own record.
needed=$("$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*/bin/llvm-readelf -d "$so" | sed -n 's/.*Shared library: \[\(.*\)\]/\1/p')
echo "libconcat_mobile.so needs: $(echo $needed)"
for lib in $needed; do
  if [ -f "$CONCAT_ANDROID_LIBS/$lib" ]; then cp "$CONCAT_ANDROID_LIBS/$lib" "$libs/"; fi
done
if echo "$needed" | grep -q 'libc++_shared.so' && [ ! -f "$libs/libc++_shared.so" ]; then
  cp "$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*/sysroot/usr/lib/$target/libc++_shared.so "$libs/"
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
