#!/usr/bin/env python3
"""The interface's string inventory, and the check that every locale keeps up.

    scripts/locales.py            # bring locales/en.json up to date with the
                                  # source, and drop lines no source asks for
    scripts/locales.py --check    # report what each locale lacks; exit 1 on
                                  # anything out of step, and on a language
                                  # the app and the READMEs do not both have
    scripts/locales.py --check --same
                                  # also list each line still in English
    scripts/locales.py --check --strict
                                  # ... and exit 1 on any of them

A locale is a JSON file of keys to words: `"export.tenBitColour": "10-bit
colour"`. A key is dotted lowerCamelCase - the area of the interface the
string belongs to (`common` for one used all over), then a name for it.

Every string a person reads passes through `I18n.t("key")` (or `t1`, `t2`,
`upper`) in the .slint tree, or `t("key")` / `tf("key", ...)` in the window's
Rust. Those keys' English is written in en.json by whoever adds the string.
Effects, filters, transitions and text presets are data, not interface:
their words are always the English their manifests write, and have no
lines here.

en.json is the inventory a translator starts from; see TRANSLATING.md.

The app speaks the languages the READMEs are written in, one for one:
every `locales/<code>.json` is compiled in through `BUILT_IN` in
src/crates/concat/src/i18n.rs and has a `docs/README.<code>.md` (English's
is README.md), and every README has its locale. `--check` fails on a
language one side has and the other does not.
"""
import json
import pathlib
import re
import sys

# A Windows console prints in its code page - cp1252, most often - which
# has no Bengali, no Persian and no CJK, and the first locale name printed
# was an UnicodeEncodeError. The words are written as UTF-8 wherever this
# runs; a stream that cannot be reconfigured (one replaced by a test
# harness) is left as it is.
for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8")
    except (AttributeError, ValueError):
        pass

ROOT = pathlib.Path(__file__).resolve().parent.parent
CRATE = ROOT / "src" / "crates" / "concat"
LOCALES = CRATE / "locales"
I18N = CRATE / "src" / "i18n.rs"
DOCS = ROOT / "docs"

# A Rust or Slint string literal, with its escapes.
LITERAL = r'"((?:[^"\\]|\\.)*)"'
SLINT_CALL = re.compile(r"I18n\.(?:t[12]?|upper)\(\s*" + LITERAL)
RUST_CALL = re.compile(r"(?<![A-Za-z_])(?:i18n::)?tf?\(\s*" + LITERAL)
KEY = re.compile(r"^[a-z][A-Za-z0-9]*(\.[a-z][A-Za-z0-9]*)+$")
# One entry of i18n.rs's BUILT_IN: `("pt-BR", include_str!("../locales/pt-BR.json")),`.
BUILT_IN_ENTRY = re.compile(r'\(\s*"([^"]+)",\s*include_str!\("\.\./locales/([^"]+)\.json"\)\s*\)')


# Lines that read the same in every language and are not a translation
# someone forgot: the app's name, units, and the names of formats and
# things.
SAME_KEYS = {
    "settings.concat",
    "controls.hertz",
    "controls.kelvin",
    "export.cbr",
    "export.vbr",
    "studio.lut",
}
# A value made of nothing but places, numbers, units and acronyms is the
# same in every language too: "{0} Hz", "LUT", "4K".
UNITS = {"Hz", "kHz", "K", "dB", "px", "fps", "ms", "s", "x", "%"}
ACRONYM = re.compile(r"^[A-Z0-9][A-Z0-9.:/+-]*$")


def same_is_fine(key: str, english: str) -> bool:
    """Whether a line identical to the English is meant to be."""
    if key in SAME_KEYS:
        return True
    words = re.sub(r"\{\d+\}", " ", english).split()
    return all(word in UNITS or ACRONYM.match(word) or not re.search(r"[A-Za-z]", word)
               for word in words)


def unescape(text: str) -> str:
    return text.replace('\\"', '"').replace("\\\\", "\\")


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
    return wanted, problems


def parity() -> list[str]:
    """Where the app's languages and the READMEs' part: a locale file the
    app does not compile in, a code compiled in with no file, a locale
    with no README, a README with no locale."""
    files = {path.stem for path in LOCALES.glob("*.json")}
    built_in: set[str] = set()
    for code, file in BUILT_IN_ENTRY.findall(I18N.read_text(encoding="utf-8")):
        built_in.add(code)
        if code != file:
            files.add(f"{code} (BUILT_IN reads {file}.json)")
    readmes = {path.name.removeprefix("README.").removesuffix(".md")
               for path in DOCS.glob("README.*.md")}
    if (ROOT / "README.md").exists():
        readmes.add("en")
    out = []
    out += [f"locales/{code}.json is not in BUILT_IN in src/crates/concat/src/i18n.rs"
            for code in sorted(files - built_in)]
    out += [f"BUILT_IN lists {code}, which has no locales/{code}.json"
            for code in sorted(built_in - files)]
    out += [f"locales/{code}.json has no docs/README.{code}.md"
            for code in sorted((files | built_in) - readmes)]
    out += [f"docs/README.{code}.md has no locales/{code}.json"
            for code in sorted(readmes - files)]
    return out


def check(wanted: dict[str, str], list_same: bool = False, strict: bool = False) -> int:
    """Reports each locale's missing, stale and untranslated lines. Stale
    fails; a line still in English is counted, listed with `list_same`, and
    fails only with `strict` - there is a backlog of them, and it is to be
    seen before it is to be enforced."""
    failed = 0
    untranslated = 0
    for path in sorted(LOCALES.glob("*.json")):
        if path.name == "en.json":
            continue
        data = read(path)
        strings = {k: v for k, v in data.items() if k != "_"}
        stale = sorted(set(strings) - set(wanted))
        missing = sorted(set(wanted) - set(strings))
        same = sorted(key for key, value in strings.items()
                      if key in wanted and value == wanted[key]
                      and not same_is_fine(key, wanted[key]))
        untranslated += len(same)
        name = data.get("_", {}).get("name", "")
        print(f"{path.stem:8s} {name:20s} {len(strings):4d} lines, "
              f"{len(missing):3d} missing, {len(stale):3d} stale, "
              f"{len(same):3d} still English")
        for key in stale:
            print(f"    stale:   {key}")
            failed = 1
        for key in missing:
            print(f"    missing: {key}  ({wanted[key]!r})")
        if list_same or strict:
            for key in same:
                print(f"    english: {key}  ({wanted[key]!r})")
    if untranslated:
        print(f"warning: {untranslated} lines are the English word for word; "
              "--same lists them, --strict fails on them")
        if strict:
            failed = 1
    return failed


def main() -> int:
    wanted, problems = inventory()
    for problem in problems:
        print(problem)
    if "--check" in sys.argv:
        apart = parity()
        for problem in apart:
            print(f"languages: {problem}")
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
        failed = 1 if problems else check(wanted, "--same" in sys.argv, "--strict" in sys.argv)
        if apart:
            print(f"languages: the app and the READMEs part on {len(apart)}; "
                  "every locale has a README and every README a locale")
        return 1 if apart else failed
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
