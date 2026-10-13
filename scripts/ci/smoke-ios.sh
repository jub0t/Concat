#!/usr/bin/env bash
# Installs the simulator build of the iOS app on an iOS Simulator, starts
# it, and fails if it does not come up and stay up: nothing else in the
# phone build ever runs what it packages.
#
#   smoke-ios.sh [--out DIR] [--wait SECS] [--device UDID] [--no-smoke] Concat.app
#
# Concat.app is the bundle scripts/ios-app.sh lays out for
# aarch64-apple-ios-sim; a device build cannot run on a simulator, and the
# script says so rather than failing at the install. On a fresh iPhone
# simulator of the newest iOS runtime Xcode has (or on UDID, which is left
# booted and kept), it
#
#   1. starts the app with `--smoke`, which runs the whole start-up - the
#      GPU, the engine, the window's component and every model handed to
#      it - as far as the window's first publish, logs "smoke: ok" and
#      exits 0; and fails unless that line comes back within 180s;
#   2. starts the app as the home screen would and watches it for WAIT
#      seconds (45 by default), failing as soon as its process is gone or
#      its output or the system log has a Rust panic (panicked at), an
#      uncaught Objective-C exception, one of the app's own start-up
#      failures (could not start, an event loop that ended in an error),
#      or a preview that failed ("Preview failed", logged as
#      `preview: ...`, #312); and failing at the end unless the app's
#      first log line (the logger's "Concat <version> · ios ...") is there.
#
# --no-smoke leaves out the first part. Whatever the outcome it leaves in
# DIR (smoke-ios by default): the app's own output from both runs, the
# system log of its process, a screenshot at the end, and any crash report
# the run left. CI uploads that folder. Runs in CI (mobile.yml) and locally
# on a Mac with Xcode; needs python3 to read simctl's lists.
set -euo pipefail

out="smoke-ios"
wait_secs=45
device=""
smoke=1

usage() {
  echo "usage: $0 [--out DIR] [--wait SECS] [--device UDID] [--no-smoke] Concat.app" >&2
  exit 2
}

while [ $# -gt 0 ]; do
  case "$1" in
    --out) out=${2:?}; shift 2 ;;
    --wait) wait_secs=${2:?}; shift 2 ;;
    --device) device=${2:?}; shift 2 ;;
    --no-smoke) smoke=0; shift ;;
    -h|--help) usage ;;
    --*) usage ;;
    *) break ;;
  esac
done
[ $# -eq 1 ] || usage
app=${1%/}
[ -d "$app" ] || { echo "smoke: no app bundle at $app" >&2; exit 2; }

say() { printf '\n== %s\n' "$*"; }

mkdir -p "$out"
created=""
launch_pid=""
log_pid=""
# Older than anything the run writes, so a crash report can be told apart
# from the ones already there.
mark=$(mktemp)

# A time limit without coreutils: perl's alarm, which the exec keeps. A
# command that overstays it dies of SIGALRM, status 142.
limit() {
  local secs=$1
  shift
  perl -e 'alarm shift; exec @ARGV or die "exec: $!"' "$secs" "$@"
}

# Everything the folder should hold, gathered on the way out whatever the
# outcome, and the simulator put back the way it was found.
cleanup() {
  set +e
  if [ -n "$device" ]; then
    xcrun simctl io "$device" screenshot "$out/screen.png" > /dev/null 2>&1 || rm -f "$out/screen.png"
    [ -z "${bundle_id:-}" ] || xcrun simctl terminate "$device" "$bundle_id" > /dev/null 2>&1
  fi
  for pid in $launch_pid $log_pid; do
    kill "$pid" 2>/dev/null
    wait "$pid" 2>/dev/null
  done
  # A crash report written since the run started, by the app's process.
  find "$HOME/Library/Logs/DiagnosticReports" -maxdepth 1 -name 'Concat*' -newer "$mark" \
    -exec cp {} "$out/" \; 2>/dev/null
  rm -f "$mark"
  if [ -n "$created" ]; then
    xcrun simctl shutdown "$created" > /dev/null 2>&1
    xcrun simctl delete "$created" > /dev/null 2>&1
  fi
  set -e
}
trap cleanup EXIT

fail() {
  for f in "$out/smoke-run.txt" "$out/console.txt"; do
    if [ -s "$f" ]; then
      echo "-- the last lines of $(basename "$f") --"
      tail -n 60 "$f"
    fi
  done
  if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::error::$*"; else echo "smoke: $*" >&2; fi
  exit 1
}

say "the bundle"
plist=$app/Info.plist
[ -f "$plist" ] || fail "no Info.plist in $app"
bundle_id=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$plist")
executable=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$plist")
[ -x "$app/$executable" ] || fail "no executable $executable in $app"
# The platform the binary was linked for, from its load commands: 7 is the
# iOS Simulator, 2 a phone.
platform=$(otool -l "$app/$executable" | awk '/LC_BUILD_VERSION/ { found = 1 } found && $1 == "platform" { print $2; exit }')
case "$platform" in
  7|IOSSIMULATOR) ;;
  *) fail "$app/$executable is built for platform ${platform:-unknown}, not the iOS Simulator (7): build for aarch64-apple-ios-sim" ;;
esac
codesign --verify --deep --strict "$app" || fail "the bundle's signature does not verify"
echo "$bundle_id, $executable, built for the iOS Simulator"

say "the simulator"
if [ -z "$device" ]; then
  # A fresh iPhone on the newest iOS runtime there is, so the run starts
  # from nothing a run before left behind.
  pick=$(python3 -I -c '
import json, subprocess, sys
def simctl(*what):
    return json.loads(subprocess.run(["xcrun", "simctl", "list", "-j", *what],
                                     check=True, capture_output=True, text=True).stdout)
runtimes = [r for r in simctl("runtimes", "available")["runtimes"]
            if r.get("platform", r["name"].split()[0]) == "iOS"]
if not runtimes:
    sys.exit("no iOS simulator runtime is installed")
runtime = max(runtimes, key=lambda r: [int(p) for p in r["version"].split(".")])
# The newest iPhone the runtime runs, and of those the plain model.
newness = {t["identifier"]: t.get("minRuntimeVersion", 0) for t in simctl("devicetypes")["devicetypes"]}
phones = [t for t in runtime.get("supportedDeviceTypes", []) if t.get("productFamily") == "iPhone"]
if not phones:
    sys.exit("the runtime " + runtime["name"] + " runs no iPhone")
phone = max(phones, key=lambda t: (newness.get(t["identifier"], 0), -len(t["name"])))
print(runtime["identifier"], phone["identifier"], runtime["name"] + ", " + phone["name"])
') || fail "could not choose a simulator"
  read -r runtime type name <<< "$pick"
  echo "a new $name"
  created=$(xcrun simctl create concat-smoke "$type" "$runtime")
  device=$created
fi
echo "device $device"
state=$(xcrun simctl list devices | grep -F "$device" || true)
case "$state" in
  *"(Booted)"*) ;;
  *) xcrun simctl boot "$device" || fail "the simulator did not boot" ;;
esac
limit 600 xcrun simctl bootstatus "$device" -b > /dev/null || fail "the simulator did not finish booting within 600s"

say "install"
limit 300 xcrun simctl install "$device" "$app" || fail "simctl install failed"

# The system's log of the app's process, for what never reaches its own
# output: UIKit's complaints, an uncaught exception, the reason it died.
xcrun simctl spawn "$device" log stream --level debug --style compact \
  --predicate "process == \"$executable\"" > "$out/oslog.txt" 2>&1 &
log_pid=$!

# The lines that mean the start-up failed: Rust's on a panic (the app's
# hook writes the standard message into its own line), Objective-C's on an
# uncaught exception, and the app's own on a start-up it gave up on, on a
# window whose event loop ended in an error, or on a preview that failed.
fatal='panicked at|Terminating app due to uncaught exception|Preview failed|(^|: )(preview: |could not start|close: event loop exited [(]ok=false[)])'

if [ "$smoke" = 1 ]; then
  say "$executable --smoke"
  status=0
  limit 180 xcrun simctl launch --console-pty --terminate-running-process "$device" "$bundle_id" --smoke \
    > "$out/smoke-run.txt" 2>&1 || status=$?
  tail -n 40 "$out/smoke-run.txt"
  [ "$status" -ne 142 ] || fail "--smoke was still running after 180s"
  if found=$(grep -E -m 5 "$fatal" "$out/smoke-run.txt"); then
    fail "--smoke failed: $(head -n 1 <<< "$found")"
  fi
  grep -q 'smoke: ok' "$out/smoke-run.txt" || fail "--smoke ended without \"smoke: ok\" (simctl exited $status)"
  # "smoke: ok" and then a crash on the way out is still a crash: v0.2.7-rc.2's
  # Linux arm64 bundle did exactly that. simctl's --console-pty ends with
  # the app's process; whether its status is the app's own is for the first
  # run to show, and anything but 0 fails either way.
  [ "$status" -eq 0 ] || fail "--smoke said \"smoke: ok\" and then simctl exited with $status"
fi

say "start $bundle_id, and watch it for ${wait_secs}s"
xcrun simctl launch --console-pty --terminate-running-process "$device" "$bundle_id" \
  > "$out/console.txt" 2>&1 &
launch_pid=$!
deadline=$((SECONDS + wait_secs))
while [ "$SECONDS" -lt "$deadline" ]; do
  sleep 2
  if found=$(cat "$out/console.txt" "$out/oslog.txt" | grep -E -m 5 "$fatal"); then
    printf '%s\n' "$found"
    fail "the app failed on start: $(head -n 1 <<< "$found")"
  fi
  # simctl's --console-pty lasts exactly as long as the app's process does.
  if ! kill -0 "$launch_pid" 2>/dev/null; then
    status=0
    wait "$launch_pid" || status=$?
    launch_pid=""
    fail "the app's process is gone $((wait_secs - deadline + SECONDS))s after the launch (simctl exited $status)"
  fi
done

say "the outcome"
# The logger's first line, written once the app's own code is running.
grep -E -q 'Concat [0-9][^ ]* · ios' "$out/console.txt" \
  || fail "the app's first log line (\"Concat <version> · ios ...\") never reached its output"
echo "-- the app's output --"
head -n 40 "$out/console.txt"
echo
echo "the app is up after ${wait_secs}s with nothing fatal in its output; see $out/"
say "ok"
