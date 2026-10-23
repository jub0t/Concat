#!/bin/sh
# Captures one app's largest on-screen window, and nothing else on the
# screen: winshot.sh <owner-name-substring> <out.png> [max-px]
set -eu
owner=$1
out=$2
max=${3:-1600}
here=$(cd "$(dirname "$0")" && pwd)
id=$(swift "$here/winid.swift" "$owner" 2>/dev/null || true)
if [ -z "$id" ]; then
    echo "no on-screen window owned by '$owner'" >&2
    exit 1
fi
screencapture -x -o -l "$id" "$out"
sips -Z "$max" "$out" >/dev/null
echo "$out"
