#!/usr/bin/env bash
# Generate a folder of square icon PNGs from a single 512x512 source.
#
# Usage:
#   scripts/generate-icons.sh [source.png] [output-dir]
#
# Defaults:
#   source     assets/concat_logo_512.png
#   output-dir assets/icons
#
# Uses sips on macOS, ImageMagick (magick/convert) elsewhere.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="${1:-$REPO_ROOT/assets/concat_logo_512.png}"
OUT="${2:-$REPO_ROOT/assets/icons}"
SIZES=(512 256 128 64 32 16)

if [[ ! -f "$SRC" ]]; then
  echo "error: source image not found: $SRC" >&2
  exit 1
fi

mkdir -p "$OUT"

resize() { # resize <size> <dest>
  if command -v sips >/dev/null 2>&1; then
    sips -z "$1" "$1" "$SRC" --out "$2" >/dev/null
  elif command -v magick >/dev/null 2>&1; then
    magick "$SRC" -resize "${1}x${1}" "$2"
  elif command -v convert >/dev/null 2>&1; then
    convert "$SRC" -resize "${1}x${1}" "$2"
  else
    echo "error: need sips or ImageMagick (magick/convert) on PATH" >&2
    exit 1
  fi
}

base="$(basename "$SRC" .png)"
base="${base%_512}"
for size in "${SIZES[@]}"; do
  dest="$OUT/${base}_${size}.png"
  resize "$size" "$dest"
  echo "wrote $dest"
done
