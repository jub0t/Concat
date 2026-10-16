#!/usr/bin/env bash
# Whether every Mach-O file in a bundle runs on the oldest OS the bundle
# says it runs on.
#
# Usage: check-min-os.sh <bundle> <declared-minimum> [--enforce]
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
set -euo pipefail

bundle="$1"
declared="$2"
enforce="${3:-}"

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

over=()
checked=0
while IFS= read -r -d '' file; do
  file -b "$file" | grep -q 'Mach-O' || continue
  checked=$((checked + 1))
  own=$(minimum_of "$file")
  [ -n "$own" ] || continue
  if [ "$own" != "$declared" ] && [ "$(newer "$own" "$declared")" = "$own" ]; then
    over+=("${file#"$bundle"/} needs $own")
  fi
done < <(find "$bundle" -type f -print0)

echo "check-min-os: $checked Mach-O files in $bundle, declared minimum $declared"
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
