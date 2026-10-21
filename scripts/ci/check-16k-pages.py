#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
"""Fails when any native library in an APK is not aligned for 16 KB pages.

A phone whose kernel uses 16 KB pages - Android 15 and later can, and
Google Play requires every app to support it - refuses to load a library
whose LOAD segments are aligned to less. v0.2.8 shipped
libconcat_android.so at 4 KB beside three 16 KB libraries.

    scripts/ci/check-16k-pages.py path/to/app.apk
"""
import struct
import sys
import zipfile

PAGE = 16384


def load_alignment(elf: bytes) -> int:
    if elf[:4] != b"\x7fELF" or elf[4] != 2:
        raise ValueError("not a 64-bit ELF")
    phoff = struct.unpack_from("<Q", elf, 0x20)[0]
    phentsize, phnum = struct.unpack_from("<HH", elf, 0x36)
    aligns = [
        struct.unpack_from("<Q", elf, phoff + i * phentsize + 0x30)[0]
        for i in range(phnum)
        if struct.unpack_from("<I", elf, phoff + i * phentsize)[0] == 1
    ]
    return min(aligns) if aligns else 0


def main() -> int:
    bad = 0
    with zipfile.ZipFile(sys.argv[1]) as apk:
        for name in apk.namelist():
            if not (name.startswith("lib/") and name.endswith(".so")):
                continue
            align = load_alignment(apk.read(name))
            ok = align >= PAGE
            bad += not ok
            print(f"{'ok ' if ok else 'BAD'} {align:>6}  {name}")
    if bad:
        print(f"{bad} librar{'y is' if bad == 1 else 'ies are'} not aligned for 16 KB pages")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
