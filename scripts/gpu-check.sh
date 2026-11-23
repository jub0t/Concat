#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jareer Abdullah
#
# The GPU checks CI cannot run: CI's Windows runners draw on WARP, the
# CPU's stand-in for a GPU, which compiles shaders unoptimised and has no
# driver to reset. Run this on a machine with a real GPU before a release,
# and after any change to the compositor, the effects or the export path:
#
#   scripts/gpu-check.sh [clip]
#
# It runs, in order, and stops at nothing - every check is reported:
#   1. the compositor's tests, GPU required (concat-render);
#   2. every built-in effect drawn at 4K within its budget, the timings shown;
#   3. the worker runtime: crash, hang, fail and cancel (concat-worker);
#   4. an export from the app's own Export sheet (--smoke-export);
#   5. the same export with the first worker made to crash, which must
#      finish on the CPU in a second worker while the editor stays up.
#
# The clip defaults to one made here with FFmpeg: 6 s of 1080p30 and a tone.
# Builds go to CARGO_TARGET_DIR (default: target/ under src/). FFmpeg's
# DLLs are put on PATH from FFMPEG_DIR, or from ~/.cargo/config.toml's.
set -uo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

ffmpeg_dir=${FFMPEG_DIR:-}
if [ -z "$ffmpeg_dir" ] && [ -f "$HOME/.cargo/config.toml" ]; then
  ffmpeg_dir=$(sed -n 's/^FFMPEG_DIR *= *"\(.*\)"/\1/p' "$HOME/.cargo/config.toml" | head -1 | sed 's#\\\\#/#g')
fi
if [ -n "$ffmpeg_dir" ] && [ -d "$ffmpeg_dir/bin" ]; then
  PATH="$ffmpeg_dir/bin:$PATH"
fi
export PATH

clip=${1:-}
if [ -z "$clip" ]; then
  clip=$work/clip.mp4
  if ! ffmpeg -hide_banner -loglevel error -y \
      -f lavfi -i testsrc2=size=1920x1080:rate=30:duration=6 \
      -f lavfi -i sine=frequency=440:duration=6 \
      -c:v mpeg4 -q:v 3 -c:a aac -shortest "$clip"; then
    echo "gpu-check: no clip given and FFmpeg could not make one" >&2
    exit 2
  fi
fi

results=()
failed=0
step() {
  local name=$1
  shift
  local log=$work/$(echo "$name" | tr ' /' '__').log
  local began=$(date +%s)
  echo "== $name"
  if "$@" >"$log" 2>&1; then
    results+=("PASS  $name  ($(( $(date +%s) - began ))s)")
  else
    results+=("FAIL  $name  ($(( $(date +%s) - began ))s)")
    failed=1
    tail -n 40 "$log"
  fi
  # What a reader wants from a run that passed: the GPU's own numbers.
  grep -E "an invert at|^ +[0-9.]+ms  |test result|smoke: |worker" "$log" | tail -n 25 || true
}

cd "$repo/src"
step "compositor tests" env CONCAT_REQUIRE_GPU=1 cargo test -p concat-render --lib
step "effects within budget" env CONCAT_REQUIRE_GPU=1 cargo test -p concat-render --lib \
  every_shader_package_draws_a_4k_frame_within_its_budget -- --nocapture
step "worker runtime" cargo test -p concat-worker
step "app build" cargo build --profile quick --bin concat
step "export" cargo run --profile quick --bin concat -- --smoke-export "$clip"
step "export, first worker crashes" env CONCAT_TEST_EXPORT_WORKER_ABORT=1 \
  cargo run --profile quick --bin concat -- --smoke-export "$clip"

echo
echo "gpu-check on $(command -v nvidia-smi >/dev/null && nvidia-smi --query-gpu=name,driver_version --format=csv,noheader || echo "this machine's GPU")"
printf '%s\n' "${results[@]}"
exit $failed
