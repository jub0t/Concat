#!/usr/bin/env bash
# ONNX Runtime for one desktop platform, laid out as Microsoft's own
# archive for it was: ./onnxruntime-PLATFORM-VERSION/lib and /include.
#
#   scripts/ci/onnxruntime.sh PLATFORM
#
# PLATFORM is linux-x64, linux-aarch64, win-x64, win-arm64 or osx-x86_64.
#
# The libraries come out of Microsoft's NuGet package (Microsoft.ML.
# OnnxRuntime), pinned in deps.lock and checked by fetch.sh. The GitHub
# release archives this used to fetch went with microsoft/onnxruntime
# itself, which has answered 404 since 2026-11-17. The package's Linux
# libraries are byte for byte the ones those archives held (v0.2.11's
# packages ship them); the rest of CI reads the same files at the same
# paths as before.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

platform=${1:-}
case "$platform" in
  linux-x64)     rid=linux-x64;   version=1.28.0; package=onnxruntime-nuget ;;
  linux-aarch64) rid=linux-arm64; version=1.28.0; package=onnxruntime-nuget ;;
  win-x64)       rid=win-x64;     version=1.28.0; package=onnxruntime-nuget ;;
  win-arm64)     rid=win-arm64;   version=1.28.0; package=onnxruntime-nuget ;;
  # 1.23.2: the last Microsoft built for Intel Macs (ORT_INTEL_MAC_VERSION).
  osx-x86_64)    rid=osx-x64;     version=1.23.2; package=onnxruntime-nuget-intel-mac ;;
  *)
    echo "usage: $0 linux-x64|linux-aarch64|win-x64|win-arm64|osx-x86_64" >&2
    exit 2
    ;;
esac

nupkg="onnxruntime-$version.nupkg"
"$here/fetch.sh" "$package" "$nupkg"

unpacked=$(mktemp -d)
trap 'rm -rf "$unpacked"' EXIT
# A .nupkg is a zip. unzip where there is one; Windows' runners have
# Python and 7-Zip in place of it, which unpack the whole of it.
python=$(command -v python3 || command -v python || true)
if command -v unzip > /dev/null 2>&1; then
  unzip -q "$nupkg" "runtimes/$rid/native/*" "build/native/include/*" -d "$unpacked"
elif [ -n "$python" ]; then
  "$python" -I -c 'import sys, zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])' \
    "$nupkg" "$unpacked"
else
  7z x -y -bd "-o$unpacked" "$nupkg" > /dev/null
fi

native="$unpacked/runtimes/$rid/native"
stem="onnxruntime-$platform-$version"
rm -rf "$stem"
mkdir -p "$stem/lib"
cp -R "$unpacked/build/native/include" "$stem/include"

case "$platform" in
  linux-*)
    # Named and linked as the archive had them: the file by its full
    # version, its soname (libonnxruntime.so.1) a link to it.
    cp "$native/libonnxruntime.so" "$stem/lib/libonnxruntime.so.$version"
    chmod 755 "$stem/lib/libonnxruntime.so.$version"
    ln -sf "libonnxruntime.so.$version" "$stem/lib/libonnxruntime.so.1"
    cp "$native/libonnxruntime_providers_shared.so" "$stem/lib/"
    ;;
  win-*)
    cp "$native"/onnxruntime.dll "$native"/onnxruntime.lib \
      "$native"/onnxruntime_providers_shared.dll "$native"/onnxruntime_providers_shared.lib \
      "$stem/lib/"
    ;;
  osx-*)
    cp "$native/libonnxruntime.dylib" "$stem/lib/libonnxruntime.$version.dylib"
    ln -sf "libonnxruntime.$version.dylib" "$stem/lib/libonnxruntime.dylib"
    ;;
esac

echo "onnxruntime.sh: $stem from $nupkg"
ls -l "$stem/lib"
