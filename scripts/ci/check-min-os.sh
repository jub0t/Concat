#!/usr/bin/env bash
# Whether every Mach-O file in a bundle runs on the oldest OS the bundle
# says it runs on.
#
# Usage: check-min-os.sh <bundle> <declared-minimum> [--enforce]
#                        [--lazy <path in the bundle>]...
#
# A bundle declares its floor once (LSMinimumSystemVersion on a Mac,
# MinimumOSVersion on a phone), but every library in it was built for a
# floor of its own, and dyld does not refuse one built for a newer system:
# it loads it, and the process dies on the first symbol the older system
# lacks. 0.2.7 for Intel Macs declared macOS 12 and shipped an ONNX Runtime
# built for 13.4, which died at launch on 12 (#333); Homebrew's libraries
# are built for the runner they were installed on.
#
# Lists each file whose own minimum is newer than the declared one, as a
# warning, in the job summary too. With --enforce, any such file fails.
#
# A --lazy file is one the app opens itself, at first use, and only on a
# system at least as new as the minimum in that file's own header - the
# Intel Mac's ONNX Runtime, which needs 13.4 where the app needs 12
# (#333; concat-vision's onnx.rs reads the same field). dyld never loads
# it at launch, so it is held against its own minimum rather than the
# bundle's: listed with the system it needs, and an error if it is not
# there, since then the features behind it are missing.
set -euo pipefail

bundle="$1"
declared="$2"
shift 2
enforce=
lazy=()
while [ "$#" -gt 0 ]; do
  case "$1" in
    --enforce) enforce=--enforce ;;
    --lazy) lazy+=("$2"); shift ;;
    *) echo "check-min-os: unknown argument $1" >&2; exit 2 ;;
  esac
  shift
done

# Whether a path in the bundle is one of the --lazy ones.
is_lazy() {
  local path
  for path in ${lazy[@]+"${lazy[@]}"}; do
    [ "$path" = "$1" ] && return 0
  done
  return 1
}

# The newer of two dotted versions.
newer() { printf '%s\n%s\n' "$1" "$2" | sort -V | tail -n 1; }

# A Mach-O's own minimum OS: LC_BUILD_VERSION's minos, or the older
# LC_VERSION_MIN_* command's version.
minimum_of() {
  otool -l "$1" 2>/dev/null | awk '
    /LC_BUILD_VERSION/ { build = 1; next }
    /LC_VERSION_MIN_/  { legacy = 1; next }
    build  && $1 == "minos"   { print $2; exit }
    legacy && $1 == "version" { print $2; exit }'
}

for path in ${lazy[@]+"${lazy[@]}"}; do
  if [ ! -f "$bundle/$path" ]; then
    echo "check-min-os: $path, loaded at first use, is not in $bundle" >&2
    exit 1
  fi
  echo "check-min-os: $path is loaded at first use, on macOS $(minimum_of "$bundle/$path") or later"
done

over=()
checked=0
while IFS= read -r -d '' file; do
  file -b "$file" | grep -q 'Mach-O' || continue
  is_lazy "${file#"$bundle"/}" && continue
  checked=$((checked + 1))
  own=$(minimum_of "$file")
  [ -n "$own" ] || continue
  if [ "$own" != "$declared" ] && [ "$(newer "$own" "$declared")" = "$own" ]; then
    over+=("${file#"$bundle"/} needs $own")
  fi
done < <(find "$bundle" -type f -print0)

echo "check-min-os: $checked Mach-O files in $bundle loaded at launch, declared minimum $declared"
if [ "${#over[@]}" -eq 0 ]; then
  echo "check-min-os: every file runs on $declared"
  exit 0
fi

level=warning
[ "$enforce" = --enforce ] && level=error
for line in "${over[@]}"; do
  if [ -n "${GITHUB_ACTIONS:-}" ]; then
    echo "::$level::$(basename "$bundle"): $line, newer than the declared $declared"
  else
    echo "check-min-os: $line, newer than the declared $declared"
  fi
done
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    printf '### %s: files newer than the declared minimum, %s\n\n' "$(basename "$bundle")" "$declared"
    printf -- '- %s\n' "${over[@]}"
    echo
  } >> "$GITHUB_STEP_SUMMARY"
fi
[ "$level" = error ] && exit 1
exit 0
