#!/usr/bin/env python3
"""The model mirror: the table, the fill, and the manifest a release carries.

    scripts/models.py --check                 # the table against the engine
    scripts/models.py mirror --out dir        # fill a mirror from upstream
    scripts/models.py release-manifest \\
        --version 0.2.1 --tag v0.2.1 \\
        --bundles dir --out manifest.json     # what a release describes

Concat fetches its cutout networks, its Kokoro voice bank and its whisper
models on demand rather than carrying them in the bundle. models/manifest.toml
says what they are and where they come from; this script is what fills the
mirror from upstream, and what turns the table into the manifest.json every
release ships beside its bundles. See that file for the shape of a row.
"""
import argparse
import hashlib
import json
import pathlib
import re
import sys
import tomllib
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "models" / "manifest.toml"
CRATES = ROOT / "engine" / "crates"

# The repository the mirror lives on, and the engine constant that must
# agree with this file about which release holds it.
REPO = "jub0t/Concat"
HOST_MODELS = CRATES / "concat-host" / "src" / "models.rs"

# Where each family's table lives in the engine, for --check.
TABLES = {
    "cutout": CRATES / "concat-vision" / "src" / "models.rs",
    "tts": CRATES / "concat-speech" / "src" / "tts.rs",
    "whisper": CRATES / "concat-speech" / "src" / "transcribe.rs",
}

# The eight bundles a release publishes, as build-app.yml and mobile.yml
# name them. Stem is formatted with the version.
BUNDLES = [
    ("macos", "arm64", "Concat-{v}-macos-arm64.dmg"),
    ("macos", "x86_64", "Concat-{v}-macos-x86_64.dmg"),
    ("linux", "x86_64", "Concat-{v}-linux-x86_64.tar.gz"),
    ("linux", "aarch64", "Concat-{v}-linux-aarch64.tar.gz"),
    ("windows", "x86_64", "Concat-{v}-windows-x86_64.zip"),
    ("windows", "aarch64", "Concat-{v}-windows-aarch64.zip"),
    ("android", "arm64", "Concat-{v}-android-arm64.apk"),
    ("ios", "arm64", "Concat-{v}-ios-arm64.ipa"),
]


def table() -> dict:
    return tomllib.loads(MANIFEST.read_text(encoding="utf-8"))


def asset_url(release: str, file: str) -> str:
    return f"https://github.com/{REPO}/releases/download/{release}/{file}"


def engine_id(model: dict) -> str:
    """What the engine calls this model.

    The engine's own tables are keyed the way each family's upstream names
    things - a cutout model by its file, a Kokoro bundle and a whisper model
    by the id their archive name is built from - so the id is read back out
    of the file rather than stored twice.
    """
    file = model["file"]
    if model["family"] == "tts":
        return file.removesuffix(".tar.bz2")
    if model["family"] == "whisper":
        return file.removeprefix("ggml-").removesuffix(".bin")
    return file


def digest(path: pathlib.Path) -> str:
    sha = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            sha.update(block)
    return sha.hexdigest()


# ── check ────────────────────────────────────────────────────────────────


def check() -> int:
    data = table()
    models = data.get("model", [])
    release = data.get("release", "")
    failed = 0

    def bad(message: str) -> None:
        nonlocal failed
        print(f"    {message}")
        failed = 1

    if not release:
        bad("the table names no release")
    if not models:
        bad("the table has no models")

    for field in ("id", "family", "file", "bytes", "sha256", "licence", "upstream"):
        for model in models:
            if field not in model:
                bad(f"{model.get('id', '?')!r} has no {field}")

    for key in ("id", "file"):
        seen: dict[str, int] = {}
        for model in models:
            seen[model.get(key, "")] = seen.get(model.get(key, ""), 0) + 1
        for value, count in seen.items():
            if count > 1:
                bad(f"{value!r} is the {key} of {count} models; a mirror is one flat release")

    for model in models:
        if model.get("family") not in TABLES:
            bad(f"{model['id']!r} is in family {model.get('family')!r}, which no engine table owns")
        if not str(model.get("upstream", "")).startswith("https://"):
            bad(f"{model['id']!r} has no https upstream")
        if model.get("bytes", 0) <= 0:
            bad(f"{model['id']!r} has no size")
        sha = model.get("sha256", "")
        if sha and not re.fullmatch(r"[0-9a-f]{64}", sha):
            bad(f"{model['id']!r} has a sha256 that is not 64 hex digits")

    # The engine constant and this file must name the same release, or the
    # app asks for a mirror that is not the one being filled.
    source = HOST_MODELS.read_text(encoding="utf-8") if HOST_MODELS.exists() else ""
    if f'pub const RELEASE: &str = "{release}";' not in source:
        bad(f"{HOST_MODELS.relative_to(ROOT)} does not name the release {release!r}")
    if f'"{REPO}"' not in source:
        bad(f"{HOST_MODELS.relative_to(ROOT)} does not name the repository {REPO!r}")

    # Every model this table carries must be one the engine offers, with the
    # same digest: the table is what the mirror is filled from, the engine's
    # constant is what a download is checked against, and a download checked
    # against a digest nothing was mirrored under can never pass.
    for family, path in TABLES.items():
        rows = [model for model in models if model.get("family") == family]
        text = path.read_text(encoding="utf-8").split("#[cfg(test)]")[0]
        ids = set(re.findall(r'^\s*(?:pub )?id: "([^"]+)"', text, re.M))
        if family == "cutout":
            ids = set(re.findall(r'^\s*file: "([^"]+)"', text, re.M))
        wanted = {engine_id(model) for model in rows}
        for missing in sorted(wanted - ids):
            bad(f"{path.relative_to(ROOT)} has no model {missing!r}")
        for extra in sorted(ids - wanted):
            bad(f"{extra!r} is in {path.relative_to(ROOT)} and not in the table")
        for model in rows:
            sha = model.get("sha256", "")
            if sha and f'sha256: "{sha}"' not in text:
                bad(f"{path.relative_to(ROOT)} does not carry the digest of {model['id']!r}")

    empty = [model["id"] for model in models if not model.get("sha256")]
    total = sum(model.get("bytes", 0) for model in models)
    print(f"{len(models)} models, {total / 1e9:.2f} GB, mirrored on {release}")
    if empty:
        # Not a failure: a model added here is unverified until the mirror
        # workflow has run once and reported what it fetched.
        print(f"    {len(empty)} awaiting a digest from the mirror: {', '.join(empty)}")
    return failed


# ── mirror ───────────────────────────────────────────────────────────────


def fill(path: pathlib.Path, block: str, sha: str, what: str) -> bool:
    """Fills one empty `sha256` inside one fenced block of `path`."""
    text = path.read_text(encoding="utf-8")
    updated, count = re.subn(block, lambda match: match.group(1) + sha + '"', text, count=1)
    if not count:
        print(f"    could not record the digest in {what}", file=sys.stderr)
        return False
    path.write_text(updated, encoding="utf-8")
    return True


def record(model: dict, sha: str) -> None:
    """Writes a digest into the table and into the engine's table beside it.

    Both, because the table is what the mirror is filled from and the
    engine's constant is what a download is checked against: a digest in
    only one of them is a download that can never pass. Each match is fenced
    to one model's own block, so the empty digest that gets filled is never
    the next one down.
    """
    model_id = model["id"]
    fill(
        MANIFEST,
        r'(\[\[model\]\]\nid = "'
        + re.escape(model_id)
        + r'"(?:(?!\[\[model\]\])[\s\S])*?sha256 = ")"',
        sha,
        str(MANIFEST.relative_to(ROOT)),
    )
    path = TABLES[model["family"]]
    item, key = ("ModelSpec", "file") if model["family"] == "cutout" else ("KnownModel", "id")
    fill(
        path,
        r"("
        + item
        + r' \{\n(?:(?!\n    \},)[\s\S])*?'
        + key
        + r': "'
        + re.escape(engine_id(model))
        + r'"(?:(?!\n    \},)[\s\S])*?sha256: ")"',
        sha,
        str(path.relative_to(ROOT)),
    )


def stream(url: str, into: pathlib.Path) -> tuple[int, str]:
    sha = hashlib.sha256()
    received = 0
    request = urllib.request.Request(url, headers={"User-Agent": f"concat-models ({REPO})"})
    with urllib.request.urlopen(request) as response, into.open("wb") as handle:
        while True:
            block = response.read(1024 * 1024)
            if not block:
                break
            handle.write(block)
            sha.update(block)
            received += len(block)
    return received, sha.hexdigest()


def mirror(out: pathlib.Path, only: list[str]) -> int:
    data = table()
    models = [m for m in data["model"] if not only or m["family"] in only or m["id"] in only]
    out.mkdir(parents=True, exist_ok=True)
    failed = 0
    for model in models:
        target = out / model["file"]
        print(f"{model['id']}: {model['upstream']}")
        partial = target.with_suffix(target.suffix + ".part")
        try:
            received, sha = stream(model["upstream"], partial)
        except Exception as error:  # noqa: BLE001 - the report is the point
            partial.unlink(missing_ok=True)
            print(f"    could not fetch: {error}", file=sys.stderr)
            failed = 1
            continue
        expected = model.get("sha256", "")
        if expected and expected != sha:
            partial.unlink(missing_ok=True)
            print(f"    expected sha256 {expected}, got {sha}", file=sys.stderr)
            failed = 1
            continue
        partial.replace(target)
        print(f"    {received} bytes, sha256 {sha}")
        if not expected:
            record(model, sha)
            print(f"    recorded in {MANIFEST.relative_to(ROOT)}")
        if received != model["bytes"]:
            # Not fatal - the table's size is for a progress bar before the
            # server says - but a large drift means the row is stale.
            print(f"    note: table says {model['bytes']} bytes, upstream sent {received}")
    return failed


# ── release manifest ─────────────────────────────────────────────────────


def release_manifest(version: str, tag: str, bundles: pathlib.Path | None) -> dict:
    data = table()
    release = data["release"]

    binaries: dict[str, dict] = {}
    for platform, arch, stem in BUNDLES:
        file = stem.format(v=version)
        entry = {
            "file": file,
            "url": asset_url(tag, file),
        }
        if bundles is not None:
            path = bundles / file
            if path.exists():
                entry["bytes"] = path.stat().st_size
                entry["sha256"] = digest(path)
            else:
                # A target that did not build is absent rather than a row
                # promising a file that is not there.
                continue
        binaries.setdefault(platform, {})[arch] = entry

    models = {}
    for model in data["model"]:
        models[model["id"]] = {
            "family": model["family"],
            "file": model["file"],
            "bytes": model["bytes"],
            "sha256": model["sha256"],
            "licence": model["licence"],
            "url": asset_url(release, model["file"]),
            "upstream": model["upstream"],
        }

    return {
        "schema": 1,
        "product": "Concat",
        "version": version,
        "tag": tag,
        "models_release": release,
        "binaries": binaries,
        "models": models,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="the table against the engine")
    sub = parser.add_subparsers(dest="command")

    fill = sub.add_parser("mirror", help="fill a mirror from upstream")
    fill.add_argument("--out", type=pathlib.Path, required=True)
    fill.add_argument("--only", action="append", default=[], help="a family or an id; repeatable")

    emit = sub.add_parser("release-manifest", help="the manifest.json a release carries")
    emit.add_argument("--version", required=True)
    emit.add_argument("--tag", required=True)
    emit.add_argument("--bundles", type=pathlib.Path)
    emit.add_argument("--out", type=pathlib.Path)

    args = parser.parse_args()
    if args.command == "mirror":
        return mirror(args.out, args.only)
    if args.command == "release-manifest":
        manifest = release_manifest(args.version, args.tag, args.bundles)
        text = json.dumps(manifest, indent=2) + "\n"
        if args.out:
            args.out.write_text(text, encoding="utf-8")
            print(f"{args.out}: {len(manifest['binaries'])} platforms, {len(manifest['models'])} models")
        else:
            print(text, end="")
        return 0
    return check()


if __name__ == "__main__":
    sys.exit(main())
