#!/usr/bin/env bash

# Build a .moon bundle for a Hebnix release.

# Usage:
#   assets/moon/build-moon-bundle.sh --stage stage/Concat-0.2.6-linux-x86_64 \
#       [--version 0.2.6] [--output FILE] [--moon PATH] [--keep-work]
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN="concat"

STAGE=""
VERSION=""
OUTPUT=""
MOON=""
KEEP_WORK=0

usage() {
    cat <<EOF
usage: build-moon-bundle.sh --stage DIR [options]

  --stage DIR    staged folder: concat, lib/, concat.desktop, concat.png
                 (what build-app.yml's "Stage (Linux)" step leaves behind)
  --version V    version for the manifest (default: src/Cargo.toml's)
  --output FILE  bundle to write (default: the stage folder's name, .moon)
  --moon PATH    moon binary (default: the one on PATH)
  --keep-work    keep the staged bundle folder and print where it is
  -h, --help     this text
EOF
}

die() {
    echo "error: $*" >&2
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        --stage) STAGE="$2"; shift 2 ;;
        --version) VERSION="$2"; shift 2 ;;
        --output) OUTPUT="$2"; shift 2 ;;
        --moon) MOON="$2"; shift 2 ;;
        --keep-work) KEEP_WORK=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1 (try --help)" ;;
    esac
done

if [ -z "$VERSION" ]; then
    VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$REPO_ROOT/src/Cargo.toml" | head -1)"
fi
[ -n "$VERSION" ] || die "cannot tell the version, pass --version"

[ -n "$STAGE" ] || die "--stage is required (try --help)"
[ -d "$STAGE" ] || die "--stage: not a folder: $STAGE"

for needed in "$BIN" "lib" "$BIN.desktop" "$BIN.png"; do
    [ -e "$STAGE/$needed" ] || die "--stage: $STAGE has no $needed"
done
[ -x "$STAGE/$BIN" ] || die "--stage: $STAGE/$BIN is not executable"
[ -d "$STAGE/lib" ] || die "--stage: $STAGE/lib is not a folder"

if ! ls "$STAGE"/lib/*.so* >/dev/null 2>&1; then
    die "--stage: $STAGE/lib has no shared libraries in it, so the bundle would not run"
fi

WORK="$(mktemp -d)"
cleanup() {
    if [ "$KEEP_WORK" -eq 1 ]; then
        echo "staged bundle kept at $WORK/bundle"
    else
        rm -rf "$WORK"
    fi
}
trap cleanup EXIT

BUNDLE="$WORK/bundle"
mkdir -p "$BUNDLE/app" "$BUNDLE/desktop" "$BUNDLE/icon"

echo "== staging bundle =="
install -Dm755 "$STAGE/$BIN" "$BUNDLE/app/$BIN"
cp -a "$STAGE/lib" "$BUNDLE/app/lib"
install -Dm644 "$STAGE/$BIN.desktop" "$BUNDLE/desktop/$BIN.desktop"
install -Dm644 "$STAGE/$BIN.png" "$BUNDLE/icon/$BIN.png"
for doc in LICENSE THIRD_PARTY_NOTICES.md; do
    if [ -f "$STAGE/$doc" ]; then
        install -Dm644 "$STAGE/$doc" "$BUNDLE/app/$doc"
    else
        echo "note: $STAGE has no $doc, the bundle will not carry it"
    fi
done

cat > "$BUNDLE/$BIN.manifest" <<EOF
dir=app
main=app/$BIN
version=$VERSION
desktop=desktop/$BIN.desktop
link=$BIN
to=app/$BIN
cmd=$BIN
EOF

find "$BUNDLE" -mindepth 1 | sed "s|^$BUNDLE|  |" | sort

OUT="${OUTPUT:-$(basename "$STAGE").moon}"
mkdir -p "$(dirname "$OUT")"
rm -f "$OUT"

if [ -n "$MOON" ]; then
    [ -x "$MOON" ] || die "--moon: not executable: $MOON"
elif command -v moon >/dev/null 2>&1; then
    MOON="$(command -v moon)"
fi

echo "== packing bundle =="
if [ -n "$MOON" ]; then
    "$MOON" bundle "$BUNDLE" "$OUT" --force
else
    echo "note: moon not found, packing with tar (same file: a .moon is a tar.gz)"
    tar -C "$BUNDLE" -czf "$OUT" .
fi
[ -f "$OUT" ] || die "no bundle was written to $OUT"

echo "== checking bundle =="
listing="$(tar -tzf "$OUT")"
for member in "$BIN.manifest" "app/$BIN" "app/lib/" "desktop/$BIN.desktop" "icon/$BIN.png"; do
    echo "$listing" | grep -q "$member" || die "the bundle has no $member"
    echo "  ok  $member"
done
echo "$listing" | grep -q "app/lib/.*\.so" || die "the bundle carries no shared library"

echo "== done =="
echo "  bundle: $OUT"
echo "  size:   $(du -h "$OUT" | cut -f1)"
echo "  sha256: $(sha256sum "$OUT" | cut -d' ' -f1)"
echo "  install it with: moon install $OUT"