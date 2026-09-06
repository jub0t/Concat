#!/usr/bin/env python3
"""The interface's string inventory, and the check that every locale keeps up.

    scripts/locales.py            # rewrite locales/en.json from the source
    scripts/locales.py --check    # report what each locale lacks; exit 1 on
                                  # a key no source asks for

Every string a person reads passes through `I18n.t("...")` in the .slint
tree or `t("...")` / `tf("...")` in the window's Rust, with the English as
the key. This script collects those keys, plus the names the effect
packages and text presets carry in their manifests (they are looked up the
same way), and writes them to en.json with each key as its own value.
That file is the inventory a translator starts from; see TRANSLATING.md.
"""
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
CRATE = ROOT / "engine" / "crates" / "concat"
LOCALES = CRATE / "locales"
PACKAGES = ROOT / "engine" / "crates" / "concat-effects" / "packages"

# A Rust or Slint string literal, with its escapes.
LITERAL = r'"((?:[^"\\]|\\.)*)"'
SLINT_CALL = re.compile(r"I18n\.t[12]?\(\s*" + LITERAL)
RUST_CALL = re.compile(r"(?<![A-Za-z_])(?:i18n::)?tf?\(\s*" + LITERAL)
TOML_FIELD = re.compile(r'^(name|description|category|label)\s*=\s*' + LITERAL, re.M)
PRESET = re.compile(r'look\(\s*"[^"]+",\s*' + LITERAL)


def unescape(text: str) -> str:
    return text.replace('\\"', '"').replace("\\\\", "\\")


def keys() -> set[str]:
    out: set[str] = set()
    for path in (CRATE / "ui").rglob("*.slint"):
        if "demo" in path.parts:
            continue
        text = "\n".join(
            line for line in path.read_text(encoding="utf-8").split("\n") if not line.lstrip().startswith("//")
        )
        for match in SLINT_CALL.finditer(text):
            out.add(unescape(match.group(1)))
    for path in (CRATE / "src").rglob("*.rs"):
        # Code only: not the comments that describe the call, and not the
        # tests, whose keys are made up.
        text = path.read_text(encoding="utf-8").split("#[cfg(test)]")[0]
        text = "\n".join(line for line in text.split("\n") if not line.lstrip().startswith("//"))
        for match in RUST_CALL.finditer(text):
            out.add(unescape(match.group(1)))
        for match in PRESET.finditer(text):
            out.add(unescape(match.group(1)))
    for manifest in PACKAGES.glob("*/effect.toml"):
        for match in TOML_FIELD.finditer(manifest.read_text(encoding="utf-8")):
            value = unescape(match.group(2))
            if value:
                out.add(value)
    # The shelf a package without a category lands on.
    out.add("Other")
    # Placeholders pass through unchanged and are not strings to translate.
    return {key for key in out if key.strip("{}0123456789 ")}


def read(path: pathlib.Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def write_inventory(inventory: set[str]) -> None:
    body = {"_": {"name": "English"}}
    for key in sorted(inventory, key=str.casefold):
        body[key] = key
    LOCALES.mkdir(exist_ok=True)
    (LOCALES / "en.json").write_text(
        json.dumps(body, ensure_ascii=False, indent=2) + "\n"
    )


def check(inventory: set[str]) -> int:
    failed = 0
    for path in sorted(LOCALES.glob("*.json")):
        if path.name == "en.json":
            continue
        data = read(path)
        strings = {k: v for k, v in data.items() if k != "_"}
        stale = sorted(set(strings) - inventory)
        missing = sorted(inventory - set(strings))
        name = data.get("_", {}).get("name", "")
        print(f"{path.stem:8s} {name:20s} {len(strings):4d} lines, "
              f"{len(missing):3d} missing, {len(stale):3d} stale")
        for key in stale:
            print(f"    stale:   {key!r}")
            failed = 1
        for key in missing:
            print(f"    missing: {key!r}")
    return failed


def main() -> int:
    inventory = keys()
    if "--check" in sys.argv:
        current = read(LOCALES / "en.json")
        listed = {k for k in current if k != "_"}
        if listed != inventory:
            print("en.json is out of date: run scripts/locales.py")
            for key in sorted(inventory - listed):
                print(f"    new:     {key!r}")
            for key in sorted(listed - inventory):
                print(f"    gone:    {key!r}")
            return 1
        return check(inventory)
    write_inventory(inventory)
    print(f"{len(inventory)} strings in {LOCALES / 'en.json'}")
    return check(inventory) and 0


if __name__ == "__main__":
    sys.exit(main())
