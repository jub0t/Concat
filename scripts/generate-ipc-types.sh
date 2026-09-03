#!/usr/bin/env bash
# Regenerates desktop/src/lib/generated/ from the Rust wire types.
#
# ts-rs exports each #[ts(export)] type when its crate's tests run with the
# `types` feature on; TS_RS_EXPORT_DIR aims all of them at one directory.
# CI runs this and fails on `git diff`, so the committed files can never
# drift from the Rust source of truth.
#
# Usage: scripts/generate-ipc-types.sh [engine|host|all]
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
export TS_RS_EXPORT_DIR="$root/desktop/src/lib/generated"

part="${1:-all}"

if [ "$part" = "engine" ] || [ "$part" = "all" ]; then
  (cd "$root/engine" && cargo test -p concat-project -p concat-export \
    --features concat-project/types,concat-export/types export_bindings)
fi

if [ "$part" = "host" ] || [ "$part" = "all" ]; then
  (cd "$root/desktop/src-tauri" && cargo test --features types export_bindings)
fi
