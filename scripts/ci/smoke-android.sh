#!/usr/bin/env bash
# Installs the built APK on an Android device or emulator, starts it the
# way the launcher does, and fails if it does not stay up: nothing else in
# the phone build ever runs what it packages.
#
#   smoke-android.sh [--out DIR] [--wait SECS] [--ready REGEX] [--no-install]
#                    [--package NAME] APK
#
# On whatever device adb talks to (ANDROID_SERIAL picks one of several):
# waits for it to finish booting; checks the device can run one of the
# ABIs the APK carries (an x86_64 emulator image runs an arm64-only APK
# through its ARM translation, and says so in ro.product.cpu.abilist);
# installs the APK; starts its launcher activity with `am start -W`; and
# then watches for WAIT seconds (45 by default). It fails as soon as
#
#   - the process is gone,
#   - logcat has, from the app, a Java crash (FATAL EXCEPTION), a native
#     one (Fatal signal), a Rust panic (panicked at), an ANR (ANR in), or
#     one of the app's own start-up failures (could not start, an event
#     loop that ended in an error, or a preview that failed: the window's
#     "Preview failed" notice, which the log carries as `preview: ...`,
#     #312),
#
# and passes if none of that has happened by the end of the wait, with the
# app's own first line ("Concat <version> starting") in the log, so a pass
# means its native code ran. With --ready, it also requires a line matching
# REGEX (an extended regular expression) by the end of the wait, and stops
# waiting once it appears.
#
# One crash is not counted, and only on a device that runs the APK through
# its ARM translation: a SIGSEGV whose crash dump puts the app's library at
# cpuinfo_arm_linux_init. ONNX Runtime's static initialisers detect the
# CPU as the library loads, and under the translation /proc/cpuinfo
# describes the x86_64 host, not an ARM core, which the detection does not
# survive. It happens before any of the app's code runs, so such a device
# cannot test the APK at all: the script says so as a warning, in the job
# summary, and exits 0 without claiming a pass. Any other crash, or this
# one on a device running the APK natively, fails as before. The
# translation before Android 12 leaves the app's frames out of the crash
# dump, and there the crash is recognised by its address and its timing
# instead (see untestable_here).
#
# Whatever the outcome it leaves in DIR (smoke-android by default): the
# whole logcat from before the launch to the end, the app's lines alone,
# a screenshot of the screen at the end, the window focus, and the
# device's properties. CI uploads that folder. The package is
# org.standalones.concat unless --package names another.
#
# --no-install starts what is already installed, which leaves a device
# somebody is using as it was apart from the restart of the app. Runs in
# CI (mobile.yml) and locally against any device adb can see. Needs adb,
# unzip and perl.
set -euo pipefail

out=smoke-android
wait_secs=45
ready=""
install=1
package=org.standalones.concat

usage() {
  echo "usage: $0 [--out DIR] [--wait SECS] [--ready REGEX] [--no-install] [--package NAME] APK" >&2
  exit 2
}

while [ $# -gt 0 ]; do
  case "$1" in
    --out) out=${2:?}; shift 2 ;;
    --wait) wait_secs=${2:?}; shift 2 ;;
    --ready) ready=${2:?}; shift 2 ;;
    --no-install) install=0; shift ;;
    --package) package=${2:?}; shift 2 ;;
    -h|--help) usage ;;
    --*) usage ;;
    *) break ;;
  esac
done
[ $# -eq 1 ] || usage
apk=$1
[ -f "$apk" ] || { echo "smoke: no APK at $apk" >&2; exit 2; }

say() { printf '\n== %s\n' "$*"; }
note() {
  if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::warning::$*"; else echo "smoke: $*"; fi
}

mkdir -p "$out"
logcat_file=$out/logcat.txt
logcat_pid=""
collected=""

# Everything the folder should hold, gathered on the way out whatever the
# outcome: a failure is exactly when the screenshot and the log are wanted.
collect() {
  collected=1
  set +e
  if [ -n "$logcat_pid" ]; then
    kill "$logcat_pid" 2>/dev/null
    wait "$logcat_pid" 2>/dev/null
    logcat_pid=""
  fi
  adb exec-out screencap -p > "$out/screen.png" 2>/dev/null || rm -f "$out/screen.png"
  {
    adb shell dumpsys activity activities 2>/dev/null | grep -i 'ResumedActivity'
    adb shell dumpsys window 2>/dev/null | grep -E 'mCurrentFocus|mFocusedApp'
  } | tr -d '\r' > "$out/focus.txt"
  adb shell getprop > "$out/getprop.txt" 2>/dev/null
  # Tombstones for a native crash, where the device lets them be read.
  adb logcat -d -b crash -v threadtime > "$out/logcat-crash.txt" 2>/dev/null
  if [ -f "$logcat_file" ]; then
    grep -E ' concat +:' "$logcat_file" > "$out/concat.txt" || true
  fi
  set -e
}
# And on a way out nobody wrote a `fail` for - a command that `set -e`
# ends the script on - so the background logcat is stopped and the folder
# still holds what the failure left.
trap '[ -n "$collected" ] || collect' EXIT

fail() {
  collect
  if [ -s "$out/concat.txt" ]; then
    echo "-- the app's last lines --"
    tail -n 60 "$out/concat.txt"
  fi
  if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::error::$*"; else echo "smoke: $*" >&2; fi
  exit 1
}

# A time limit without coreutils, so this runs on a Mac too: perl's alarm,
# which the exec keeps. A command that overstays it dies of SIGALRM, 142.
limit() {
  local secs=$1
  shift
  perl -e 'alarm shift; exec @ARGV or die "exec: $!"' "$secs" "$@"
}

say "the device"
limit 300 adb wait-for-device || fail "no device came up for adb within 300s"
booted=""
for _ in $(seq 1 150); do
  booted=$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r' || true)
  [ "$booted" = 1 ] && break
  sleep 2
done
[ "$booted" = 1 ] || fail "the device did not finish booting within 300s"
sdk=$(adb shell getprop ro.build.version.sdk | tr -d '\r')
release=$(adb shell getprop ro.build.version.release | tr -d '\r')
abilist=$(adb shell getprop ro.product.cpu.abilist | tr -d '\r')
model=$(adb shell getprop ro.product.model | tr -d '\r')
echo "$model, Android $release (API $sdk), ABIs $abilist"

# The ABIs the APK carries native code for, from its lib/ folders. One the
# device lists is one it can load, natively or through its translation.
apk_abis=$(unzip -Z1 "$apk" | sed -n 's#^lib/\([^/]*\)/.*#\1#p' | sort -u | tr '\n' ' ')
echo "the APK carries: ${apk_abis:-no native code}"
runnable=""
for abi in $apk_abis; do
  case ",$abilist," in *",$abi,"*) runnable=$abi; break ;; esac
done
[ -n "$runnable" ] || [ -z "$apk_abis" ] || fail "the device runs $abilist and the APK carries only $apk_abis: an x86_64 emulator image needs ARM translation (API 30+ google_apis or default images have it, ATD images do not)"
primary=${abilist%%,*}
if [ -n "$runnable" ] && [ "$runnable" != "$primary" ]; then
  echo "the app runs as $runnable through the device's translation for $primary"
fi

if [ "$install" = 1 ]; then
  say "install"
  # -g grants every runtime permission the manifest asks for, so no
  # permission prompt stands between the launch and the window.
  limit 300 adb install -r -g "$apk" < /dev/null \
    || fail "adb install failed (an installed build signed with another key has to be uninstalled first)"
fi
adb shell pm path "$package" > /dev/null 2>&1 || fail "$package is not installed"

# The activity the launcher would start, asked of the package manager
# rather than written down: cargo-apk names it, not this script.
activity=$(adb shell cmd package resolve-activity --brief -c android.intent.category.LAUNCHER "$package" 2>/dev/null \
  | tr -d '\r' | tail -n 1)
case "$activity" in
  "$package"/*) ;;
  *) fail "no launcher activity for $package (resolve-activity said: $activity)" ;;
esac

say "start $activity"
# A cold start: the app stopped, the log emptied and made big enough to
# hold the whole run, and read into a file from before the launch on, so
# the app's first line cannot have scrolled out of the device's buffer by
# the time it is looked for.
adb shell am force-stop "$package" || true
adb logcat -G 16M > /dev/null 2>&1 || true
adb logcat -c || true
adb logcat -v threadtime > "$logcat_file" 2>&1 &
logcat_pid=$!
started=$(limit 180 adb shell am start -W -n "$activity" 2>&1 | tr -d '\r') \
  || fail "am start did not return within 180s"
printf '%s\n' "$started"
grep -q '^Status: ok' <<< "$started" || fail "am start did not report the launch as ok"

# The lines that mean the start-up failed, wherever they come from: the
# runtime's crash report, the C library's on a fatal signal, Rust's on a
# panic (the app's hook writes the standard message into its own line),
# the system's on an ANR, and the app's own on a start-up it gave up on or
# a window whose event loop ended in an error.
# Only the app's count: a line written by one of its processes, or one
# that names it (the system's ANR report does). An emulator's own services
# crash now and then while it settles, and that is not this app failing.
# The app's lines read "concat  : <module>: <message>".
fatal='FATAL EXCEPTION|Fatal signal [0-9]+|panicked at|ANR in |Preview failed'
fatal+='| concat +: ([A-Za-z0-9_:]+: )?(preview: |could not start|close: event loop exited [(]ok=false[)])'

# Every process the app has had since the launch: the ones the activity
# manager reports starting, which a process that dies at once still has,
# and the one running now.
# Sets `running` to the process there is now, empty when there is none,
# and adds it to `pids`. Called in this shell, not in $( ): it sets both.
pids=""
running=""
app_pids() {
  running=$(adb shell pidof "$package" 2>/dev/null | tr -d '\r' || true)
  pids=$(
    {
      sed -n "s/.*Start proc \([0-9][0-9]*\):$package\/.*/\1/p" "$logcat_file"
      printf '%s %s\n' "$running" "$pids" | tr ' ' '\n'
    } | grep -E '^[0-9]+$' | sort -u | tr '\n' ' '
  )
}

# The fatal lines that are the app's, from the log so far. threadtime puts
# the writing process's id in the third column.
fatal_lines() {
  awk -v re="$fatal" -v pids=" $pids " -v pkg="$package" '
    $0 ~ re && (index(pids, " " $3 " ") || index($0, pkg)) { print; if (++n == 5) exit }
  ' "$logcat_file"
}

# Ends the script without a verdict when the fatal lines in `found` are the
# translation's crash in ONNX Runtime's CPU detection (see the top of this
# file), and returns otherwise. The crash dump reaches the crash buffer a
# few seconds after the signal, so it is waited for.
untestable_here() {
  [ -n "$runnable" ] && [ "$runnable" != "$primary" ] || return 0
  [ -n "$found" ] || return 0
  ! grep -v -q -E 'Fatal signal 11 ' <<< "$found" || return 0
  local frame="#00 pc [0-9a-f]+ +/data/app/[^ ]*$package-[^ ]*/lib/[^ ]+[.]so [(]cpuinfo_arm_linux_init[+]"
  # The translation before Android 12 writes no frames of the app's own
  # into the crash dump, only the translator's code, which sits in
  # anonymous memory. There the crash is known by everything else it has:
  # every fatal line the same SIGSEGV at address 0x8, the top frame in the
  # translator, and not one line from the app, whose first comes at the
  # start of android_main, after the library has loaded.
  local blind="#00 pc [0-9a-f]+ +<anonymous:[0-9a-f]+>"
  local hit="" crashes=""
  for _ in $(seq 1 10); do
    crashes=$(adb logcat -d -b crash -v threadtime 2>/dev/null || true)
    hit=$(grep -E -m 1 -- "$frame" <<< "$crashes" || true)
    [ -n "$hit" ] && break
    sleep 2
  done
  local seen="its crash dump puts the fault in ONNX Runtime's CPU detection"
  if [ -z "$hit" ]; then
    ! grep -v -q -E 'Fatal signal 11 .*fault addr 0x8 ' <<< "$found" || return 0
    ! grep -E -q ' concat +:' "$logcat_file" || return 0
    hit=$(grep -E -m 1 -- "$blind" <<< "$crashes" || true)
    [ -n "$hit" ] || return 0
    seen="its crash dump names no frame of the app's (this translation writes none), and the crash matches the one the newer translation names: a SIGSEGV at 0x8 as the library loads, before the app's first line"
  fi
  collect
  printf '%s\n' "$hit"
  local why="NOT TESTED on API $sdk: the APK runs here through ARM translation ($runnable on $primary), where ONNX Runtime's CPU detection crashes as the library loads, before any of the app's code; $seen. Start it on a real $runnable device."
  note "$why"
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    printf '### Android smoke, API %s: not tested\n\n%s\n' "$sdk" "$why" >> "$GITHUB_STEP_SUMMARY"
  fi
  say "not tested"
  exit 0
}

say "watch for ${wait_secs}s"
seen_ready=""
deadline=$((SECONDS + wait_secs))
while [ "$SECONDS" -lt "$deadline" ]; do
  sleep 2
  app_pids
  found=$(fatal_lines)
  if [ -n "$found" ]; then
    printf '%s\n' "$found"
    untestable_here
    fail "the app failed on start: $(head -n 1 <<< "$found")"
  fi
  if [ -z "$running" ]; then
    # The log can trail the death by a moment; the reason is worth the wait.
    gone=$((wait_secs - deadline + SECONDS))
    sleep 3
    found=$(fatal_lines)
    [ -z "$found" ] || printf '%s\n' "$found"
    untestable_here
    fail "the app's process is gone ${gone}s after the launch${found:+: $(head -n 1 <<< "$found")}"
  fi
  if [ -n "$ready" ] && grep -E -q -- "$ready" "$logcat_file"; then
    seen_ready=1
    echo "the ready line is in the log: $(grep -E -m 1 -- "$ready" "$logcat_file")"
    break
  fi
done

say "the outcome"
collect
cat "$out/focus.txt" || true
# A process can outlive its window: an android_main that returns, on an
# error it logged as anything else, finishes the activity and leaves the
# process cached. The activity in front is the app's, or it did not start.
grep -i 'ResumedActivity' "$out/focus.txt" | grep -q "$package/" \
  || fail "the app's activity is not the one in front after ${wait_secs}s"
# The app's own first line, from android_main: proof its native code ran,
# and not only that a process by its name exists.
grep -E -q ' concat +: ([A-Za-z0-9_:]+: )?Concat .* starting' "$logcat_file" \
  || fail "the app's first line (\"Concat <version> starting\") never reached logcat"
if [ -n "$ready" ] && [ -z "$seen_ready" ]; then
  fail "no line matching '$ready' within ${wait_secs}s"
fi
if ! grep -E 'mCurrentFocus' "$out/focus.txt" | grep -q "$package"; then
  note "the window in focus is not the app's: $(grep -E 'mCurrentFocus' "$out/focus.txt" | tr -s ' ')"
fi
echo "-- the app's lines --"
head -n 40 "$out/concat.txt" || true
echo
echo "the app is up after ${wait_secs}s with nothing fatal in the log; see $out/"
say "ok"
