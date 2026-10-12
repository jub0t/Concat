#!/usr/bin/env bash
# Downloads one of the prebuilt dependencies scripts/ci/deps.lock names and
# checks it against the digest written there, so a build only ever links
# or ships bytes somebody has looked at once. A file whose digest does not
# match is deleted and the script fails, naming both digests: a release
# that was replaced upstream, or a download that went wrong, stops the
# build rather than ending up in a bundle.
#
#   scripts/ci/fetch.sh NAME OUT     download NAME to the file OUT
#   scripts/ci/fetch.sh --url NAME   print the URL deps.lock has for NAME
#
# OUT already there with the right digest is kept, so a rerun on a warm
# machine downloads nothing. Runs on Linux, macOS and Git's bash on
# Windows: sha256sum where there is one, shasum where there is not.
#
# To bump a dependency, change its row and put the new file's digest in
# it; scripts/ci/fetch.sh prints the digest it got when they differ.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
lock="$here/deps.lock"

usage() {
  echo "usage: $0 NAME OUT | $0 --url NAME" >&2
  exit 2
}

# The row for a name: its URL and digest, space-separated.
row() {
  awk -v name="$1" '
    /^[[:space:]]*(#|$)/ { next }
    $1 == name { print $2, $3; found = 1; exit }
    END { exit !found }
  ' "$lock"
}

digest() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{ print $1 }'
  else
    shasum -a 256 "$1" | awk '{ print $1 }'
  fi
}

[ $# -eq 2 ] || usage

if [ "$1" = "--url" ]; then
  found=$(row "$2") || { echo "fetch.sh: no row named $2 in $lock" >&2; exit 1; }
  echo "${found%% *}"
  exit 0
fi

name=$1
out=$2
found=$(row "$name") || { echo "fetch.sh: no row named $name in $lock" >&2; exit 1; }
url=${found%% *}
want=${found##* }

if [ -f "$out" ] && [ "$(digest "$out")" = "$want" ]; then
  echo "fetch.sh: $name is already at $out"
  exit 0
fi

echo "fetch.sh: $name from $url"
mkdir -p "$(dirname "$out")"
curl -fsSL --retry 5 --retry-all-errors --connect-timeout 30 -o "$out" "$url"

got=$(digest "$out")
if [ "$got" != "$want" ]; then
  rm -f "$out"
  {
    echo "fetch.sh: $name does not match deps.lock"
    echo "  url:      $url"
    echo "  expected: $want"
    echo "  got:      $got"
  } >&2
  exit 1
fi
echo "fetch.sh: $name verified (sha256 $got)"
