#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
#
# Copies the effect cards a run of Concat drew on a real GPU into
# src/crates/concat/cards/, one <package id>.jpg each, which the binary
# carries: what a shelf shows until a machine has drawn its own (#416).
#
# Run the app once on the build being bundled, let the Effects, Filters
# and Transitions shelves fill in, quit, then run this. With no argument it
# reads the card folder of this machine's app data.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/src/crates/concat/cards"
case "$(uname -s)" in
  Darwin) default="$HOME/Library/Application Support/org.standalones.concat/cards" ;;
  Linux) default="${XDG_DATA_HOME:-$HOME/.local/share}/org.standalones.concat/cards" ;;
  *) default="${LOCALAPPDATA:-}/org.standalones.concat/cards" ;;
esac
from="${1:-$default}"
[ -d "$from" ] || { echo "no card folder at $from" >&2; exit 1; }

mkdir -p "$out"
rm -f "$out"/*.jpg
count=0
for card in "$from"/*-????????????????.jpg; do
  [ -f "$card" ] || continue
  name="$(basename "$card")"
  id="${name%-*}"
  # Only the built-ins: a user's own package has no folder here.
  [ -d "$root/src/crates/concat-effects/packages/$id" ] || continue
  cp "$card" "$out/$id.jpg"
  count=$((count + 1))
done
echo "bundled $count cards into $out ($(du -sh "$out" | cut -f1))"
