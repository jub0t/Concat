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

changes=$(git log "$since..$until" --no-merges --format='%s' 2>/dev/null \
  | grep -Ev '^(Update [^ ]+\.(rs|slint|toml|md|yml)|Lock the flake|Format the workspace|Changelog for|Merge )' \
  | grep -Ev 'in the (export|pool) tests$' \
  | grep -Eiv '^(test|chore|ci|build|refactor|style|docs)(\([^)]*\))?!?: ' \
  | grep -Eiv "${type}.*(clippy|lint|rustfmt|(unused|duplicate|missing) [A-Za-z]* ?import|non-existent|does not compile|before test module)" \
  | grep -Eiv '^(updates?|wip|fixes?|cleanup)\.?$' \
  | sed -E "s/${type}//" \
  | awk '{ print toupper(substr($0, 1, 1)) substr($0, 2) }' \
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

Every bundle runs with nothing else installed.

- **macOS**: open the `.dmg` and drag Concat to Applications. If macOS says
  the app cannot be checked, right-click it and choose Open once.
- **Windows**: run the `-setup.exe`; it puts Concat in the Start menu and
  can take it out again. The `.msi` is the same program for an
  administrator to deploy, machine-wide.
- **Linux**: install the `.deb` or the `.rpm` with your package manager,
  or `chmod +x` the `.AppImage` and run it; no install needed.
- **Android**: open the `.apk` on the phone and allow the install from
  this source; Android 8.0 or newer, 64-bit.
- **iOS / iPadOS**: sideload the `.ipa` with AltStore, Sideloadly or
  Xcode's Devices window, which sign it with your Apple ID; iOS 15 or
  newer.

`SHA256SUMS` lists each file's checksum.

### Licences

Concat is AGPL-3.0-or-later with a plugin exception
([LICENSE-EXCEPTIONS.md](https://github.com/jub0t/Concat/blob/main/LICENSE-EXCEPTIONS.md)).
The bundles carry FFmpeg (GPL, with x264), compile in whisper.cpp (MIT) and
link sherpa-onnx with espeak-ng (GPL-3.0); Slint is used under its GPL-3.0
option. Sources and licences:
[THIRD_PARTY_NOTICES.md](https://github.com/jub0t/Concat/blob/main/THIRD_PARTY_NOTICES.md).
EOF
