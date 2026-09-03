#!/usr/bin/env bash
# The notes for a release or a nightly, from the commits it is made of.
#
# Usage: release-notes.sh <since-ref> <until-ref> <title> [nightly]
#
# Writes markdown to stdout: what changed, as the subjects of the commits
# between the two refs; how to install each bundle; the checksum file; the
# licences. The commits are the changelog - each subject on main is written
# as a sentence a user can read - so there is no second file to keep in
# step with them. Housekeeping commits (formatting, lock files, "Update
# foo.rs") are left out, and a subject that appears twice appears once.
set -euo pipefail

since="$1"
until="$2"
title="$3"
kind="${4:-release}"

changes=$(git log "$since..$until" --no-merges --format='%s' 2>/dev/null \
  | grep -Ev '^(Update [^ ]+\.(rs|slint|toml|md|yml)|Lock the flake|Format the workspace|Changelog for|Merge )' \
  | grep -Ev 'in the (export|pool) tests$' \
  | awk '!seen[$0]++' \
  | sed 's/^/- /')

echo "## $title"
echo
if [ "$kind" = "nightly" ]; then
  echo "The newest main, rebuilt on every push. For a release, see the tagged ones."
else
  echo "A self-contained build for every platform Concat ships on."
fi
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
| Windows | | `windows-x86_64.zip` | `windows-aarch64.zip` |
| Linux | | `linux-x86_64.tar.gz` | `linux-aarch64.tar.gz` |

Every bundle runs with nothing else installed.

- **macOS**: open the `.dmg` and drag Concat to Applications. If macOS says
  the app cannot be checked, right-click it and choose Open once.
- **Windows**: unzip and run `concat.exe`. The FFmpeg libraries beside it
  stay beside it.
- **Linux**: `tar xzf` the archive and run `./concat` from the folder;
  `concat.desktop` is there for a launcher.

`SHA256SUMS` lists each file's checksum.

### Licences

Concat is AGPL-3.0-or-later with a plugin exception
([LICENSE-EXCEPTIONS.md](https://github.com/jub0t/Concat/blob/main/LICENSE-EXCEPTIONS.md)).
The bundles carry FFmpeg (GPL, with x264), compile in whisper.cpp (MIT) and
link sherpa-onnx with espeak-ng (GPL-3.0); Slint is used under its GPL-3.0
option. Sources and licences:
[THIRD_PARTY_NOTICES.md](https://github.com/jub0t/Concat/blob/main/THIRD_PARTY_NOTICES.md).
EOF
