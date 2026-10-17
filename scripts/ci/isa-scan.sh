#!/usr/bin/env bash
# Scans x86_64 binaries for instructions an Ivy Bridge does not have, so a
# bundle that would die at launch with an illegal instruction on a 2012
# machine (#193, #313) fails its build instead.
#
#   scripts/ci/isa-scan.sh FILE_OR_FOLDER...
#
# Every PE (.exe, .dll) and ELF file found is disassembled with
# llvm-objdump, and each instruction from a set the x86_64 baseline the
# bundles promise (AVX, nothing newer; see GGML_* in build-app.yml) does not
# include is counted, by class:
#
#   avx2       256-bit integer vector work, permutes, broadcasts, gathers
#   fma        fused multiply-add (FMA3)
#   bmi        the BMI1 and BMI2 bit operations a compiler emits for ordinary
#              shifts and masks when told the CPU has them (shlx, shrx,
#              sarx, bzhi, andn, bextr, blsi, blsr, blsmsk, pdep, pext):
#              the sign of code compiled for Haswell as a whole rather
#              than one dispatched function. #313's crash was a shlx.
#   mulx/rorx  the two BMI2 instructions hand-written cryptography uses on
#              its fast paths (ring's, Go's), behind a CPU check; counted
#              apart so they do not drown the column above
#   avx512     anything touching a zmm or mask register
#
# Not every one of those is a fault waiting to happen. A library that asks
# the CPU what it has at run time and picks its kernels by the answer
# carries AVX2 code it never runs on an older machine: FFmpeg's codecs and
# scalers, ONNX Runtime's MLAS, sherpa-onnx. whisper's CPU kernels are one
# module per CPU generation, of which ggml loads the one the CPU can run,
# so the Haswell-and-newer modules are allowed what the rest are not. And a statically linked binary
# carries such code too - memchr, aho-corasick, ring and Skia all dispatch
# this way - so a count of zero is not what to expect from concat itself.
# scripts/ci/isa-allow.txt says, per file, how much of each class is
# expected: `any` for a library that dispatches (not even disassembled),
# or a limit per class for a binary that holds some dispatched code among
# its own - a number, past which the scan fails; `~` and a number, past
# which it warns; or `-`, no limit. A file with no row may hold none.
#
# What the count catches is the size of the problem #313 was: pyke's
# static ONNX Runtime 1.28.0 for Windows, compiled for Haswell, holds some
# 2,700 compiler-made BMI instructions (1,559 shlx, 783 shrx, 273 sarx);
# Microsoft's onnxruntime.dll of the same version holds none. What the
# count cannot do is say whether an instruction runs, which is why the
# build also launches the bundle under Intel's emulator posing as an Ivy
# Bridge (scripts/ci/smoke-*). The two together are the check.
#
# llvm-objdump is found as $LLVM_OBJDUMP, as llvm-objdump on the path, or as
# the newest llvm-objdump-<version> on the path, which is how Ubuntu installs
# it. Exits 1 when a file is over a hard limit, 2 on a usage error.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
allow="$here/isa-allow.txt"

if [ $# -eq 0 ]; then
  echo "usage: $0 FILE_OR_FOLDER..." >&2
  exit 2
fi

objdump=${LLVM_OBJDUMP:-}
if [ -z "$objdump" ]; then
  if command -v llvm-objdump >/dev/null 2>&1; then
    objdump=llvm-objdump
  else
    objdump=$(compgen -c llvm-objdump- | sort -t- -k3 -n | tail -1 || true)
  fi
fi
if [ -z "$objdump" ] || ! command -v "$objdump" >/dev/null 2>&1; then
  echo "isa-scan: no llvm-objdump; set LLVM_OBJDUMP or install LLVM" >&2
  exit 2
fi

# The allowance for a file, as "avx2 fma bmi mulx/rorx avx512" limits, or
# `any`: the first row whose pattern matches its name, or all zero.
allowance() {
  local base=$1 pattern rest
  while read -r pattern rest; do
    case "$pattern" in '' | '#'*) continue ;; esac
    # shellcheck disable=SC2254 # the pattern is a glob on purpose
    case "$base" in $pattern) echo "${rest%%#*}"; return ;; esac
  done < "$allow"
  echo 0 0 0 0 0
}

# Whether a file is an x86_64 PE or ELF, by its magic bytes and machine
# field, without needing `file` (Git's bash on Windows has none).
is_x86_64() {
  local f=$1 magic
  magic=$(od -An -tx1 -N4 "$f" 2>/dev/null | tr -d ' \n')
  case "$magic" in
    7f454c46)
      # ELF: e_machine at 18, 0x3e is x86-64.
      [ "$(od -An -tx1 -j18 -N1 "$f" | tr -d ' \n')" = 3e ]
      ;;
    4d5a*)
      # PE: the header's offset at 0x3c, then "PE\0\0" and the machine,
      # 0x8664 for x86-64.
      local off
      off=$(od -An -tu4 -j60 -N4 "$f" | tr -d ' \n')
      [ "$(od -An -tx1 -j$((off + 4)) -N2 "$f" | tr -d ' \n')" = 6486 ]
      ;;
    *) return 1 ;;
  esac
}

# Counts per class for one file, as "avx2 fma bmi mulx/rorx avx512", and
# the ten most frequent offending mnemonics after them on separate lines.
count() {
  "$objdump" -d --no-show-raw-insn "$1" 2>/dev/null | awk '
    # The mnemonic is the first word after the address; AT&T syntax,
    # which is llvm-objdump'"'"'s default for x86.
    /^ *[0-9a-f]+:/ {
      line = $0
      sub(/^ *[0-9a-f]+:[ \t]*/, "", line)
      split(line, w, /[ \t]+/)
      m = w[1]
      if (m == "" || m ~ /^(rep|lock|notrack|data16|cs|ds)$/) m = w[2]
      cls = ""
      if (line ~ /%zmm|%k[0-7]/) cls = "avx512"
      else if (m ~ /^vf(n)?m(add|sub|addsub|subadd)(132|213|231)/) cls = "fma"
      else if (m ~ /^(andn|bextr|blsi|blsmsk|blsr|bzhi|pdep|pext|sarx|shlx|shrx)[lq]?$/) cls = "bmi"
      else if (m ~ /^(mulx|rorx)[lq]?$/) cls = "asm"
      else if (m ~ /^(vpbroadcast[bwdq]|vbroadcasti128|vperm2i128|vpermd|vpermq|vpermps|vpermpd|vinserti128|vextracti128|vpmaskmov[dq]|vpsllv[dq]|vpsrlv[dq]|vpsravd|vpgather|vgather|vpblendd)/) cls = "avx2"
      else if (m ~ /^vp/ && line ~ /%ymm/ && m !~ /^(vpermil|vptest|vperm2f128)/) cls = "avx2"
      if (cls != "") { n[cls]++; seen[m]++ }
    }
    END {
      printf "%d %d %d %d %d\n", n["avx2"], n["fma"], n["bmi"], n["asm"], n["avx512"]
      for (k in seen) printf "%d %s\n", seen[k], k
    }' | { read -r head; echo "$head"; sort -rn | head -10; }
}

files=()
for arg in "$@"; do
  if [ -d "$arg" ]; then
    while IFS= read -r -d '' f; do files+=("$f"); done \
      < <(find "$arg" -type f \( -name '*.exe' -o -name '*.dll' -o -name '*.so' -o -name '*.so.*' -o -perm -u+x \) -print0)
  else
    files+=("$arg")
  fi
done

status=0
row() { printf '%-36s %10s %10s %10s %10s %10s  %s\n' "$@"; }
row file avx2 fma bmi mulx/rorx avx512 verdict
for f in "${files[@]}"; do
  is_x86_64 "$f" || continue
  base=$(basename "$f")
  read -r -a limits <<< "$(allowance "$base")"
  if [ "${limits[0]}" = any ]; then
    row "$base" - - - - - "dispatches at run time; not scanned"
    continue
  fi
  result=$(count "$f")
  read -r -a counts <<< "$(head -1 <<< "$result")"
  verdict=ok
  shown=()
  for i in 0 1 2 3 4; do
    limit=${limits[$i]:-0}
    shown+=("${counts[$i]}/$limit")
    case "$limit" in
      -) ;;
      "~"*)
        if [ "${counts[$i]}" -gt "${limit#"~"}" ] && [ "$verdict" = ok ]; then verdict=WARN; fi
        ;;
      *)
        if [ "${counts[$i]}" -gt "$limit" ]; then verdict=OVER; status=1; fi
        ;;
    esac
  done
  row "$base" "${shown[@]}" "$verdict"
  if [ "$verdict" != ok ]; then
    tail -n +2 <<< "$result" | sed 's/^/      /'
  fi
  if [ "$verdict" = WARN ] && [ -n "${GITHUB_ACTIONS:-}" ]; then
    echo "::warning::isa-scan: $base holds more post-Ivy Bridge instructions than scripts/ci/isa-allow.txt expects (${shown[*]}); see that file"
  fi
done

if [ "$status" -ne 0 ]; then
  echo
  echo "isa-scan: a file holds more post-Ivy Bridge instructions than scripts/ci/isa-allow.txt allows it." >&2
  echo "A library compiled for Haswell (AVX2, FMA, BMI2) as a whole crashes the app on older CPUs; see #313." >&2
fi
exit "$status"
