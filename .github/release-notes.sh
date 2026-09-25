#!/usr/bin/env bash
# The notes for a release, from the commits it is made of.
#
# Usage: release-notes.sh <since-ref> <until-ref> <title>
#
# Writes markdown to stdout: what changed, as the subjects of the commits
# between the two refs; how to install each bundle; the checksum file; the
# licences. The commits are the changelog - each subject on main is written
# as a sentence a user can read - so there is no second file to keep in
# step with them.
#
# What a release note is for is what somebody running the app can notice, so
# three kinds of subject are dropped on the way past. Housekeeping, by its
# shape: formatting, lock files, "Update foo.rs". Anything typed as work on
# the tree rather than on the product - a conventional-commit `test:`,
# `chore:`, `ci:`, `build:`, `refactor:`, `style:` or `docs:` says so
# itself. And a fix that only got the build green again: a lint appeased, an
# import removed, a file moved to please a compiler is not a fix anybody
# asked for.
#
# A subject that survives is printed as a sentence. A `feat(studio):` in
# front of one is scaffolding for the log and reads as noise in a release,
# so the type comes off and the first letter goes up. A subject that appears
# twice appears once.
set -euo pipefail

since="$1"
until="$2"
title="$3"

# A conventional-commit type, with its optional (scope) and breaking `!`.
type='^[a-z]+(\([^)]*\))?!?: '

# Each line opens with one mark for what it touches, from the commit's
# scope where it has one and its type where it has not - a fix reads as a
# fix at a glance, and the text rows stand apart from the timeline's. A
# subject with neither carries no mark rather than a wrong one.
marks='text=📝 speech=🗣️ enhance=🗣️ keyframes=🎞️ effects=✨ render=✨ colour=✨ color=✨ timeline=✂️ media=📁 export=📤 settings=⚙️ i18n=🌍 locales=🌍 api=🤖 server=🤖 android=📱 ios=📱 models=📦 inspector=🖥️ window=🖥️ workspace=🖥️ ui=🖥️ start=🖥️ feat=✨ fix=🐛 perf=⚡'
changes=$(git log "$since..$until" --no-merges --format='%s' 2>/dev/null \
  | grep -Ev '^(Update [^ ]+\.[a-z]+|Version [0-9]|Lock the flake|Format the workspace|Changelog for|Merge )' \
  | grep -Ev 'in the (export|pool) tests$' \
  | grep -Eiv '^(test|chore|ci|build|refactor|style|docs)(\([^)]*\))?!?: ' \
  | grep -Eiv "${type}.*(clippy|lint|rustfmt|(unused|duplicate|missing) [A-Za-z]* ?import|non-existent|does not compile|before test module|green again)" \
  | grep -Eiv '^(updates?|wip|fixes?|cleanup)\.?$' \
  | awk -v marks="$marks" '
      BEGIN {
        n = split(marks, pairs, " ")
        for (i = 1; i <= n; i++) { split(pairs[i], kv, "="); mark[kv[1]] = kv[2] }
      }
      {
        prefix = ""
        if (match($0, /^[a-z]+(\([^)]*\))?!?: /)) {
          head = substr($0, 1, RLENGTH)
          $0 = substr($0, RLENGTH + 1)
          kind = head; sub(/[(!:].*/, "", kind)
          scope = ""
          if (match(head, /\([^)]*\)/)) scope = substr(head, RSTART + 1, RLENGTH - 2)
          if (scope in mark) prefix = mark[scope] " "
          else if (kind in mark) prefix = mark[kind] " "
        }
        print prefix toupper(substr($0, 1, 1)) substr($0, 2)
      }' \
  | awk '!seen[$0]++' \
  | sed 's/^/- /')

echo "## $title"
echo
echo "A self-contained build for every platform Concat ships on."
echo
if [ -n "$changes" ]; then
  echo "### What changed"
  echo
  echo "$changes"
  echo
fi
cat <<'EOF'
### Download

| | Apple silicon | Intel / x86_64 | arm64 |
|---|---|---|---|
| macOS | `macos-arm64.dmg` | `macos-x86_64.dmg` | |
| Windows | | `windows-x86_64-setup.exe`, `.msi` | `windows-aarch64-setup.exe`, `.msi` |
| Linux | | `linux-x86_64.deb`, `.rpm`, `.AppImage` | `linux-aarch64.deb`, `.rpm`, `.AppImage` |
| Android | | | `android-arm64.apk` |
| iOS / iPadOS | | | `ios-arm64.ipa` |

`SHA256SUMS` lists each file's checksum.
EOF
