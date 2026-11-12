#!/usr/bin/env python3
"""Turns the failures in cargo logs into GitHub annotations.

    python3 scripts/ci/annotate.py src/clippy.log src/build.log ...

A job's log is only readable signed in; its annotations are public, on the
check run, through the API and on the commit. So the step that fails tees
cargo's output to a file, and a failure-only step runs this over the files:
every compiler error (clippy's denied lints included), each build script
that panicked, and each test that failed becomes an annotation, with its
file and line where the compiler named one and the first lines of the
message as its body.

GitHub keeps ten annotations of each level per step, so the first ten
failures are errors, the next ten warnings and the next ten notices, and a
last notice counts what did not fit. Missing files are skipped: a step that
failed before writing its log is not this script's to report.
"""
import os
import re
import sys

# Annotations carry test names and messages, emoji among them; a Windows
# runner's console is cp1252, which cannot write them.
for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8")
    except (AttributeError, ValueError):
        pass

ROOT = "src/"  # cargo runs in src/, annotations name paths from the repository root
BODY = 18  # lines of each message kept in its annotation
LEVELS = ["error", "warning", "notice"]
PER_LEVEL = 10

header = re.compile(r"^(error|warning)(\[[A-Za-z0-9]+\])?: (.+)$")
location = re.compile(r"^\s*--> (.+?):(\d+):(\d+)")
test_failed = re.compile(r"^\s*(?:FAIL|TIMEOUT|SIGSEGV|SIGABRT)\s+\[.*?\]\s+(.+)$")
panicked = re.compile(r"panicked at (.+?):(\d+):(\d+)")
ansi = re.compile(r"\x1b\[[0-9;]*m")


def escape(text, prop=False):
    text = text.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
    if prop:
        text = text.replace(":", "%3A").replace(",", "%2C")
    return text


def failures(lines):
    """(title, file, line, body) for every failure in one log."""
    found = []
    i = 0
    while i < len(lines):
        line = lines[i]
        hit = header.match(line)
        if hit and hit.group(1) == "error" and not hit.group(3).startswith(("aborting due to", "could not compile")):
            block = [line]
            j = i + 1
            while j < len(lines) and not header.match(lines[j]) and not test_failed.match(lines[j]):
                block.append(lines[j])
                j += 1
            path, number = None, None
            for b in block:
                where = location.match(b)
                if where:
                    path, number = where.group(1).replace("\\", "/"), where.group(2)
                    break
            if path is None:
                for b in block:
                    where = panicked.search(b)
                    if where:
                        path, number = where.group(1).replace("\\", "/"), where.group(2)
                        break
            if path and not path.startswith(("/", ROOT)) and not re.match(r"^[A-Za-z]:/", path):
                path = ROOT + path
            found.append((hit.group(3)[:200], path, number, "\n".join(block[:BODY])))
            i = j
            continue
        hit = test_failed.match(line)
        if hit:
            block = [line]
            j = i + 1
            # nextest prints the failing test's output after its FAIL line.
            while j < len(lines) and len(block) < 60 and not test_failed.match(lines[j]):
                block.append(lines[j])
                j += 1
            path, number = None, None
            for b in block:
                where = panicked.search(b)
                if where:
                    path, number = where.group(1).replace("\\", "/"), where.group(2)
                    if not path.startswith(("/", ROOT)) and not re.match(r"^[A-Za-z]:/", path):
                        path = ROOT + path
                    break
            # The panic's own message follows its "panicked at" line, up to a
            # blank line or the note about backtraces: that is the line that
            # says what was wrong, so it is kept with the assertion's sides.
            keep = []
            for k, b in enumerate(block):
                if "panicked at" in b:
                    keep.append(b)
                    for after in block[k + 1 : k + 9]:
                        if not after.strip() or after.strip().startswith(("note:", "stack backtrace")):
                            break
                        keep.append(after)
                elif b.strip().startswith(("left:", "right:")) and b not in keep:
                    keep.append(b)
            body = "\n".join([line] + keep[:BODY])
            found.append((f"test failed: {hit.group(1).strip()[:160]}", path, number, body))
            i = j
            continue
        i += 1
    return found


def main(paths):
    seen = set()
    every = []
    for path in paths:
        try:
            with open(path, encoding="utf-8", errors="replace") as log:
                lines = [ansi.sub("", l.rstrip("\n")) for l in log]
        except FileNotFoundError:
            continue
        for title, file, line, body in failures(lines):
            key = (title, file, line)
            if key not in seen:
                seen.add(key)
                every.append((title, file, line, f"{os.path.basename(path)}\n{body}"))
    for n, (title, file, line, body) in enumerate(every[: PER_LEVEL * len(LEVELS)]):
        level = LEVELS[n // PER_LEVEL]
        props = [f"title={escape(title, True)}"]
        if file:
            props.insert(0, f"file={escape(file, True)}")
            if line:
                props.insert(1, f"line={line}")
        print(f"::{level} {','.join(props)}::{escape(body)}")
    shown = min(len(every), PER_LEVEL * len(LEVELS))
    names = ", ".join(os.path.basename(p) for p in paths)
    print(f"::notice title=annotate.py::{len(every)} failures in {names}; {shown} shown")


if __name__ == "__main__":
    main(sys.argv[1:])
