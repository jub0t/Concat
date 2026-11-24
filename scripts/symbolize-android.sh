#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jareer Abdullah
#
# Names the frames of a phone's hang or crash that stand in Concat's own
# library, by function: CI's phone build is linked without line tables, so
# a frame inlined into another is named as the function it was inlined into.
#
#   symbolize-android.sh LIBRARY [TRACE]
#
# LIBRARY is the unstripped libconcat_mobile.so (libconcat_android.so before
# 0.2.13) of the build the trace
# came from: the "symbols-concat-<version>-<target>" artifact of that
# build's "Build for phones" run (kept 90 days). TRACE is a Concat log, an
# ANR dump or a tombstone, or standard input when it is left out. Every
# line naming the engine library with a "pc" is read, and the offset after
# "pc" is looked up in LIBRARY.
#
# The trace's BuildId (when it gives one) is checked against LIBRARY's: a
# library from another build names the wrong functions, confidently.
#
# llvm-symbolizer and llvm-readelf are the NDK's (ANDROID_NDK_HOME, or the
# newest NDK under the SDK), or the ones on PATH.
set -euo pipefail

library=${1:?usage: symbolize-android.sh LIBRARY [TRACE]}
trace=${2:--}
[ -f "$library" ] || { echo "no library at $library" >&2; exit 1; }

tool() {
  local name=$1 sdk ndk found
  sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}
  ndk=${ANDROID_NDK_HOME:-$(find "$sdk/ndk" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort -V | tail -1 || true)}
  found=$(find "$ndk/toolchains/llvm/prebuilt" -path "*/bin/$name" 2>/dev/null | head -1 || true)
  if [ -n "$found" ]; then
    echo "$found"
  elif command -v "$name" >/dev/null; then
    command -v "$name"
  else
    echo "no $name: install the NDK or LLVM" >&2
    exit 1
  fi
}
symbolizer=$(tool llvm-symbolizer)
readelf=$(tool llvm-readelf)

text=$(cat "$trace")
ours=$(printf '%s\n' "$text" | grep -E 'libconcat_(mobile|android)\.so' | grep -E '(^|[^[:alnum:]])pc [0-9a-fA-F]+' || true)
if [ -z "$ours" ]; then
  echo "no frame in libconcat_mobile.so (or libconcat_android.so) in the trace" >&2
  exit 1
fi

have=$("$readelf" -n "$library" | sed -n 's/.*Build ID: *\([0-9a-fA-F]*\).*/\1/p' | head -1)
wanted=$(printf '%s\n' "$ours" | sed -n 's/.*BuildId: *\([0-9a-fA-F]*\).*/\1/p' | head -1)
if [ -n "$wanted" ] && [ "$wanted" != "$have" ]; then
  echo "the trace is of build $wanted and the library is ${have:-of none}: use that build's symbols" >&2
  exit 1
fi

# Each frame under the name of its thread, as the trace has them: a
# thread's block opens with its quoted name.
printf '%s\n' "$text" | awk '
  /^[[:space:]]*"/ { sub(/^[[:space:]]+/, ""); thread = $0; next }
  /libconcat_(mobile|android)\.so/ && match($0, /(^|[^[:alnum:]])pc [0-9a-fA-F]+/) {
    pc = substr($0, RSTART, RLENGTH); sub(/.*pc /, "", pc)
    frame = "#?"; if (match($0, /#[0-9]+ pc/)) { frame = substr($0, RSTART, RLENGTH - 3) }
    if (thread != shown) { print "T " thread; shown = thread }
    print "F " frame " " pc
  }' | while read -r kind rest; do
  if [ "$kind" = T ]; then
    echo "$rest"
    continue
  fi
  read -r frame pc <<< "$rest"
  echo "  $frame 0x$pc"
  "$symbolizer" --obj="$library" --demangle --inlines --relativenames "0x$pc" \
    | sed '/^$/d; s/^/      /'
done
