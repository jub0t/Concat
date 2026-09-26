#!/usr/bin/env python3
"""The interface's string inventory, and the check that every locale keeps up.

    scripts/locales.py            # bring locales/en.json up to date with the
                                  # source, and drop lines no source asks for
    scripts/locales.py --check    # report what each locale lacks; exit 1 on
                                  # anything out of step

A locale is a JSON file of keys to words: `"export.tenBitColour": "10-bit
colour"`. A key is dotted lowerCamelCase - the area of the interface the
string belongs to (`common` for one used all over), then a name for it.

Every string a person reads passes through `I18n.t("key")` (or `t1`, `t2`,
`upper`) in the .slint tree, or `t("key")` / `tf("key", ...)` in the window's
Rust. Those keys' English is written in en.json by whoever adds the string.
The words an effect package or a text preset carries in its manifest are
looked up by keys made from its id and read in English from the manifest,
so this script writes those lines of en.json itself:

    effects.<name>.name / .description    a package's name and tooltip
    effects.categories.<shelf>            the shelf it sits on
    effects.groups.<group>                a knob group's subhead
    effects.labels.<label>                a knob's label
    presets.<name>                        a text preset's name

en.json is the inventory a translator starts from; see TRANSLATING.md.
"""
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
CRATE = ROOT / "src" / "crates" / "concat"
LOCALES = CRATE / "locales"
PACKAGES = ROOT / "src" / "crates" / "concat-effects" / "packages"

# A Rust or Slint string literal, with its escapes.
LITERAL = r'"((?:[^"\\]|\\.)*)"'
SLINT_CALL = re.compile(r"I18n\.(?:t[12]?|upper)\(\s*" + LITERAL)
RUST_CALL = re.compile(r"(?<![A-Za-z_])(?:i18n::)?tf?\(\s*" + LITERAL)
TOML_FIELD = re.compile(r'^(name|description|category|label|group)\s*=\s*' + LITERAL, re.M)
PRESET = re.compile(r'look\(\s*"([^"]+)",\s*' + LITERAL)
KEY = re.compile(r"^[a-z][A-Za-z0-9]*(\.[a-z][A-Za-z0-9]*)+$")


def unescape(text: str) -> str:
    return text.replace('\\"', '"').replace("\\\\", "\\")


def key_part(text: str) -> str:
    """A name as one segment of a key, as `i18n::key_part` makes it: its
    words in lowerCamelCase, `{0}`-style places and apostrophes left out,
    `&` read as "and"."""
    text = re.sub(r"\{\d+\}", " ", text).replace("'", "").replace("’", "")
    words = [w for w in re.split(r"[^A-Za-z0-9]+", text.replace("&", " and ")) if w]
    if not words:
        return ""
    return words[0].lower() + "".join(w[:1].upper() + w[1:].lower() for w in words[1:])


def live(text: str, rust: bool):
    """Code only: not the comments that describe a call, and not the tests,
    whose keys are made up."""
    if rust:
        text = text.split("#[cfg(test)]")[0]
    return "\n".join("" if line.lstrip().startswith("//") else line for line in text.split("\n"))


def code_keys() -> set[str]:
    out: set[str] = set()
    for path in (CRATE / "ui").rglob("*.slint"):
        if "demo" in path.parts:
            continue
        out.update(unescape(m.group(1)) for m in SLINT_CALL.finditer(live(path.read_text(encoding="utf-8"), False)))
    for path in (CRATE / "src").rglob("*.rs"):
        out.update(unescape(m.group(1)) for m in RUST_CALL.finditer(live(path.read_text(encoding="utf-8"), True)))
    # A literal that is only a place to fill - `I18n.t1("{0}", ...)` - is
    # passed through and is not a string to translate.
    return {key for key in out if key.strip("{}0123456789 ")}


def manifest_strings() -> dict[str, str]:
    """The keys the window makes from manifests and presets, with their English."""
    out: dict[str, str] = {}
    for manifest in sorted(PACKAGES.glob("*/effect.toml")):
        name = manifest.parent.name.removeprefix("concat.")
        part = key_part(name.replace("-", " "))
        for field, literal in TOML_FIELD.findall(manifest.read_text(encoding="utf-8")):
            value = unescape(literal)
            if not value:
                continue
            key = {
                "name": f"effects.{part}.name",
                "description": f"effects.{part}.description",
                "category": f"effects.categories.{key_part(value)}",
                "group": f"effects.groups.{key_part(value)}",
                "label": f"effects.labels.{key_part(value)}",
            }[field]
            out[key] = value
    # The shelf a package without a category lands on.
    out["effects.categories.other"] = "Other"
    presets = (CRATE / "src" / "presets.rs").read_text(encoding="utf-8")
    for preset, literal in PRESET.findall(presets):
        out[f"presets.{key_part(preset.removeprefix('concat.').replace('-', ' '))}"] = unescape(literal)
    return out


def read(path: pathlib.Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def write(path: pathlib.Path, data: dict) -> None:
    meta = {"_": data["_"]} if "_" in data else {}
    body = {**meta, **{key: data[key] for key in sorted(k for k in data if k != "_")}}
    path.write_text(json.dumps(body, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def inventory() -> tuple[dict[str, str], list[str]]:
    """en.json as the source has it, and what cannot be made so: a key used
    in code whose English nobody has written, or a literal in a call that
    is not a key."""
    current = read(LOCALES / "en.json")
    problems = []
    wanted: dict[str, str] = {}
    for key in sorted(code_keys()):
        if not KEY.match(key):
            problems.append(f"not a key: {key!r} - calls take a key such as \"common.export\"")
        elif key in current:
            wanted[key] = current[key]
        else:
            problems.append(f"no English: {key!r} - add it to en.json")
    for key, english in manifest_strings().items():
        if not KEY.match(key):
            problems.append(f"not a key: {key!r}, made from a manifest")
        wanted[key] = english
    return wanted, problems


def check(wanted: dict[str, str]) -> int:
    failed = 0
    for path in sorted(LOCALES.glob("*.json")):
        if path.name == "en.json":
            continue
        data = read(path)
        strings = {k: v for k, v in data.items() if k != "_"}
        stale = sorted(set(strings) - set(wanted))
        missing = sorted(set(wanted) - set(strings))
        name = data.get("_", {}).get("name", "")
        print(f"{path.stem:8s} {name:20s} {len(strings):4d} lines, "
              f"{len(missing):3d} missing, {len(stale):3d} stale")
        for key in stale:
            print(f"    stale:   {key}")
            failed = 1
        for key in missing:
            print(f"    missing: {key}  ({wanted[key]!r})")
    return failed


def main() -> int:
    wanted, problems = inventory()
    for problem in problems:
        print(problem)
    if "--check" in sys.argv:
        current = read(LOCALES / "en.json")
        listed = {k: v for k, v in current.items() if k != "_"}
        if listed != wanted:
            print("en.json is out of date: run scripts/locales.py")
            for key in sorted(set(wanted) - set(listed)):
                print(f"    new:     {key}")
            for key in sorted(set(listed) - set(wanted)):
                print(f"    gone:    {key}")
            for key in sorted(k for k in set(wanted) & set(listed) if wanted[k] != listed[k]):
                print(f"    changed: {key}")
            return 1
        return 1 if problems else check(wanted)
    write(LOCALES / "en.json", {"_": {"name": "English"}, **wanted})
    print(f"{len(wanted)} strings in {LOCALES / 'en.json'}")
    # A line for a string the interface no longer has is dropped: the
    # translation has nothing left to be of.
    for path in sorted(LOCALES.glob("*.json")):
        if path.name == "en.json":
            continue
        data = read(path)
        stale = [k for k in data if k != "_" and k not in wanted]
        for key in stale:
            del data[key]
        write(path, data)
        if stale:
            print(f"{path.stem:8s} dropped {len(stale)}: {', '.join(stale)}")
    check(wanted)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
