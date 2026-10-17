#!/usr/bin/env bash
# Launches a built Linux bundle the way a person would, and fails if it
# does not start. Nothing else in the build ever runs what it packages:
# a library left out of lib/, a dependency the .deb forgot to name, or an
# instruction an older CPU lacks all used to reach a release unnoticed.
#
#   smoke-linux.sh [--sde SDE64] DIR
#       The staged folder (DIR/concat beside DIR/lib), on this machine:
#       every library resolves from the folder alone; `concat --version`
#       exits 0; `concat --smoke` (start-up through the first frame of the
#       window, then out) exits 0 under Xvfb, and does not so much as try
#       to open the NVIDIA driver's CUDA or NVENC libraries, which nothing
#       should touch before a person asks for a hardware export. With
#       --sde, `--version` again under Intel's emulator posing as an Ivy
#       Bridge (AVX, no AVX2), the oldest CPU the x86_64 bundle promises,
#       and `--whisper-cpu`, loading whisper's CPU kernels, posing as a
#       Sandy Bridge.
#
#   smoke-linux.sh --container IMAGE DEB APPIMAGE
#       The packages, in a clean IMAGE (ubuntu:22.04, debian:12) through
#       docker: the .deb installed by apt, which pulls in exactly the
#       dependencies it declares and nothing else, then `--version` and
#       `--smoke` from the installed package and from the AppImage.
#
# Runs in CI after staging (build-app.yml), and locally against a folder
# or packages built the same way. Needs xvfb-run; strace if it is there.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

say() { printf '\n== %s\n' "$*"; }
fail() {
  if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::error::$*"; else echo "smoke: $*" >&2; fi
  exit 1
}

# A fresh home for each run, so the start-up is a first start-up: no
# settings, no recent projects, nothing cached from a run before.
fresh_home() {
  local home
  home=$(mktemp -d)
  mkdir -p "$home/.config" "$home/.cache" "$home/.local/share"
  echo "$home"
}

# Runs a command with a time limit, prints what it wrote, and fails with
# what it is told when the command does not exit 0.
expect_ok() {
  local what=$1 secs=$2 out status=0
  shift 2
  out=$(timeout --kill-after=10 "$secs" "$@" 2>&1) || status=$?
  printf '%s\n' "$out" | tail -n 60
  if [ "$status" -eq 124 ] || [ "$status" -eq 137 ]; then
    fail "$what: still running after ${secs}s"
  elif [ "$status" -ne 0 ]; then
    fail "$what: exited with $status"
  fi
}

host() {
  local sde="" dir
  if [ "${1:-}" = "--sde" ]; then sde=$2; shift 2; fi
  [ $# -eq 1 ] || { echo "usage: $0 [--sde SDE64] DIR" >&2; exit 2; }
  dir=$(cd "$1" && pwd)
  local bin="$dir/concat"
  [ -x "$bin" ] || fail "$bin is not there"

  # The build's own environment points at the FFmpeg it linked; the
  # folder has to stand without it.
  unset LD_LIBRARY_PATH

  say "every library resolves from the folder"
  local report
  report=$(ldd "$bin" || true)
  if grep -i 'not found' <<< "$report"; then
    echo "$report"
    fail "the binary does not resolve every library from its own folder"
  fi

  say "concat --version"
  HOME=$(fresh_home) expect_ok "concat --version" 60 "$bin" --version

  say "concat --smoke"
  local trace
  trace=$(mktemp)
  local tracer=()
  if command -v strace >/dev/null 2>&1; then
    tracer=(strace -f -qq -e "trace=open,openat" -o "$trace")
  else
    echo "no strace: the start-up is run but its file opens are not checked"
  fi
  HOME=$(fresh_home) expect_ok "concat --smoke" 180 \
    xvfb-run -a -s "-screen 0 1920x1080x24" "${tracer[@]}" "$bin" --smoke
  if [ ${#tracer[@]} -gt 0 ]; then
    local cuda
    cuda=$(grep -E 'lib(cuda|nvcuvid|nvidia-encode)\.so' "$trace" || true)
    if [ -n "$cuda" ]; then
      echo "$cuda"
      fail "concat --smoke tried to load the NVIDIA driver's CUDA or NVENC libraries at start-up"
    fi
    echo "no CUDA or NVENC library opened at start-up"
  fi

  if [ -n "$sde" ]; then
    say "concat --version on an emulated Ivy Bridge"
    # The emulator stops the program at the first instruction the chip
    # it poses as does not have, and says which.
    HOME=$(fresh_home) expect_ok "concat --version under SDE -ivb" 600 "$sde" -ivb -- "$bin" --version

    # whisper's CPU kernels, one module per CPU generation: ggml opens
    # every one to score it against the CPU before it keeps the best, so
    # the Haswell and Ice Lake modules are loaded here too, on a Sandy
    # Bridge - AVX without even F16C, like #318's i7-2600 - which must
    # come out with the sandybridge module and no illegal instruction.
    say "concat --whisper-cpu on an emulated Sandy Bridge"
    HOME=$(fresh_home) expect_ok "concat --whisper-cpu under SDE -snb" 600 "$sde" -snb -- "$bin" --whisper-cpu
  fi
}

container() {
  [ $# -eq 3 ] || { echo "usage: $0 --container IMAGE DEB APPIMAGE" >&2; exit 2; }
  local image=$1 deb appimage
  deb=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")
  appimage=$(cd "$(dirname "$3")" && pwd)/$(basename "$3")
  [ -f "$deb" ] || fail "$deb is not there"
  [ -f "$appimage" ] || fail "$appimage is not there"
  say "in a clean $image"
  docker run --rm \
    -v "$here/smoke-linux.sh:/ci/smoke-linux.sh:ro" \
    -v "$deb:/pkg/concat.deb:ro" \
    -v "$appimage:/pkg/concat.AppImage:ro" \
    "$image" bash /ci/smoke-linux.sh --inside /pkg/concat.deb /pkg/concat.AppImage
}

# Inside the container. Xvfb and Mesa's software drivers stand in for
# the desktop's display and GPU, and one font for the desktop's fonts;
# everything else the program needs comes from the package's own
# dependencies, so a library it forgot to name is missing here too.
inside() {
  local deb=$1 appimage=$2
  export DEBIAN_FRONTEND=noninteractive
  say "installing the .deb and what it depends on"
  apt-get update -qq
  apt-get install -y -qq --no-install-recommends \
    "$deb" xvfb xauth libgl1-mesa-dri mesa-vulkan-drivers fonts-dejavu-core > /dev/null

  say "concat --version (the package)"
  HOME=$(fresh_home) expect_ok "concat --version from the .deb" 60 concat --version
  say "concat --smoke (the package)"
  HOME=$(fresh_home) expect_ok "concat --smoke from the .deb" 180 \
    xvfb-run -a -s "-screen 0 1920x1080x24" concat --smoke

  # No FUSE in a container: the AppImage unpacks itself and runs.
  export APPIMAGE_EXTRACT_AND_RUN=1
  say "concat --version (the AppImage)"
  HOME=$(fresh_home) expect_ok "concat --version from the AppImage" 120 "$appimage" --version
  say "concat --smoke (the AppImage)"
  HOME=$(fresh_home) expect_ok "concat --smoke from the AppImage" 240 \
    xvfb-run -a -s "-screen 0 1920x1080x24" "$appimage" --smoke
}

case "${1:-}" in
  --container) shift; container "$@" ;;
  --inside) shift; inside "$@" ;;
  *) host "$@" ;;
esac
say "ok"
