#!/usr/bin/env bash
# Launches the built macOS bundle the way a person would, and fails if it
# does not start: nothing else in the build ever runs what it packages.
#
#   smoke-macos.sh Concat.app
#
# The signature holds over the whole bundle (an ad-hoc one, which is what
# a fork's build gets, passes and is reported); no binary in it loads a
# library from Homebrew or anywhere else outside the bundle; `concat
# --version` exits 0; and `concat --smoke` - start-up through the first
# frame of the window, then out - exits 0. Runs in CI after signing
# (build-app.yml), and locally against a bundle built the same way.
set -euo pipefail

say() { printf '\n== %s\n' "$*"; }
note() {
  if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::warning::$*"; else echo "smoke: $*"; fi
}
fail() {
  if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::error::$*"; else echo "smoke: $*" >&2; fi
  exit 1
}

# A time limit without coreutils: perl's alarm, which the exec keeps.
# A run that overstays it dies of SIGALRM, status 142.
expect_ok() {
  local what=$1 secs=$2 out status=0
  shift 2
  out=$(perl -e 'alarm shift; exec @ARGV or die "exec: $!"' "$secs" "$@" 2>&1) || status=$?
  printf '%s\n' "$out" | tail -n 60
  if [ "$status" -eq 142 ]; then
    fail "$what: still running after ${secs}s"
  elif [ "$status" -ne 0 ]; then
    fail "$what: exited with $status"
  fi
}

[ $# -eq 1 ] || { echo "usage: $0 Concat.app" >&2; exit 2; }
app=$1
bin="$app/Contents/MacOS/concat"
[ -x "$bin" ] || fail "$bin is not there"

say "the signature"
codesign --verify --deep --strict --verbose=2 "$app" || fail "the bundle's signature does not verify"
details=$(codesign -dv "$app" 2>&1 || true)
if grep -q 'Signature=adhoc' <<< "$details"; then
  note "the bundle is ad-hoc signed: it opens, with Gatekeeper's warning, and is not notarised"
else
  grep -E '^(Authority|TeamIdentifier)=' <<< "$details" || true
fi

say "nothing loads from outside the bundle"
leaks=""
while IFS= read -r -d '' f; do
  if [[ $(file -b "$f") == *Mach-O* ]]; then
    found=$(otool -L "$f" | tail -n +2 | grep -E '/opt/homebrew|/usr/local|/opt/local' || true)
    if [ -n "$found" ]; then leaks+="$f:"$'\n'"$found"$'\n'; fi
  fi
done < <(find "$app/Contents" -type f -print0)
if [ -n "$leaks" ]; then
  printf '%s' "$leaks"
  fail "a binary in the bundle loads a library from outside it"
fi
echo "every load path is the system's or the bundle's"

say "concat --version"
HOME=$(mktemp -d) expect_ok "concat --version" 60 "$bin" --version

say "concat --smoke"
HOME=$(mktemp -d) expect_ok "concat --smoke" 180 "$bin" --smoke

say "ok"
