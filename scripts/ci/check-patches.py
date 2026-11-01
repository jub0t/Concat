#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
"""Fails when the workspace's [patch.crates-io] stops applying.

src/Cargo.toml builds two crates from src/patches/ rather than crates.io -
Slint's Skia renderer, and whisper-rs-sys - each the published version
with one change re-applied (see src/patches/README.md). A patch whose
version no longer satisfies what depends on it is dropped by cargo with a
warning and nothing else, and the build then links the crates.io version,
change and all. This reads the lockfile cargo resolved: a package from a
local path carries no `source` in Cargo.lock, and one cargo fell back to
crates.io for carries a registry one.

    scripts/ci/check-patches.py
"""
import pathlib
import sys

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
MANIFEST = ROOT / "src" / "Cargo.toml"
LOCK = ROOT / "src" / "Cargo.lock"

# The crates [patch.crates-io] builds from src/patches/, and where each
# patch lives. Keep beside the section itself in src/Cargo.toml.
PATCHED = {
    "i-slint-renderer-skia": "patches/i-slint-renderer-skia",
    "whisper-rs-sys": "patches/whisper-rs-sys",
}


def main() -> int:
    manifest = tomllib.loads(MANIFEST.read_text(encoding="utf-8"))
    lock = tomllib.loads(LOCK.read_text(encoding="utf-8"))
    declared = manifest.get("patch", {}).get("crates-io", {})
    resolved = {}
    for package in lock.get("package", []):
        if package["name"] in PATCHED:
            resolved.setdefault(package["name"], []).append(package)

    failed = 0
    for name, where in PATCHED.items():
        entry = declared.get(name)
        if not (isinstance(entry, dict) and entry.get("path") == where):
            print(f"BAD  [patch.crates-io] does not build {name} from src/{where}")
            failed = 1
        found = resolved.get(name, [])
        if len(found) != 1:
            print(f"BAD  Cargo.lock resolves {name} {len(found)} times")
            failed = 1
            continue
        source = found[0].get("source", "")
        if source and not source.startswith("path"):
            print(f"BAD  {name} {found[0]['version']} resolved from {source}")
            failed = 1
        else:
            print(f"ok   {name} {found[0]['version']} from {source or 'the local path'}")

    if failed:
        print(
            "a patch has stopped applying: copy the version the lock resolves "
            "into src/patches/, re-apply the change, and read it again "
            "(see src/patches/README.md)"
        )
    return failed


if __name__ == "__main__":
    sys.exit(main())
