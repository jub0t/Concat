# Making extras: the package format

An **extra** is optional content that a person adds to Concat after installing
it. It can be effects, filters, transitions, sound effects, fonts, title
presets, images or sounds. It ships as one `.concatpkg` file and is offered by a
**repository** through a signed `index.json`.

Extras are **data**. Nothing in one is ever run as a program. Shaders are
compiled and trialled by the effects catalogue the same way a hand-made effect
package is, and a font is read by the text engine. An extra cannot carry
executables, scripts or native libraries, and the format has no hook that
would run one.

A complete, working example is in
[`extras-example/example.starter-pack`](extras-example/example.starter-pack).
It is tested on every build (`concat-host` →
`the_documented_example_installs_and_loads`).

## 1. Lay out a folder

```
my-pack/
  extra.toml                      the manifest (below)
  README.md                       optional: README.md, CHANGELOG.md, LICENSE[.txt|.md],
                                  NOTICE[.txt], icon.png, preview.png/.jpg may sit at the top
  effects/                        a root, handed to one kind
    me.warm-glow/                 an effect package, exactly as Concat's own
      effect.toml
      effect.wgsl
  fonts/
    Inter-Bold.ttf
```

Every other file must sit under a root named in `[[provides]]`, and must be a
type that kind takes:

| `kind` | What it is for | File types |
|---|---|---|
| `effects` | Effect packages (filters, effects, transitions, generators, sound effects): one folder each with `effect.toml`, as in `src/crates/concat-effects/packages/` | toml, wgsl, cube, png, jpg, jpeg, webp, md, txt |
| `fonts` | Fonts for the font picker and titles | ttf, otf, ttc, md, txt |
| `title-presets` | Title presets: `*.toml`, or folders with `preset.toml` and their font | toml, ttf, otf, ttc, png, md, txt |
| `stickers` | Pictures for the Stickers page's **Pictures** shelf, placed over the picture | png, jpg, jpeg, webp, md, txt |
| `sounds` | Sound files | wav, mp3, flac, ogg, opus, m4a, aac, aif, aiff, md, txt |

Effect package ids follow the effects catalogue's rules: `author.name`, and
never under `concat.` (that author belongs to the built-ins). An id that
another installed package already uses is refused at install.

## 2. Write `extra.toml`

```toml
format = 1

[extra]
id = "me.starter-pack"          # author.name: lowercase letters, digits, . and -
name = "Starter Pack"
version = "1.0.0"               # semantic version
summary = "One line for the card."
description = "Paragraphs for the extra's page."
author = "Me"
licence = "CC0-1.0"             # the licence of the contents (SPDX id where there is one)
homepage = "https://example.com"   # https only, optional
category = "effects"            # effects | transitions | fonts | titles | stickers | audio | other
min_app = "0.2.11"              # optional: the oldest Concat it works in
platforms = []                  # optional, rarely needed: e.g. ["windows-x86_64"]
keywords = ["glow", "wipe"]
provides_ids = []               # optional: extra ids a project can name (effect ids are found automatically)

[[provides]]
kind = "effects"
root = "effects"

[[requires]]                    # optional, repeatable
id = "me.base-looks"
version = "^1.2"                # semver range; empty = any version
reason = "Shares its colour tables."
```

You do not write the `[[file]]` list. `pack` writes it, with every file's
size and SHA-256.

Limits: up to 20,000 files, one file up to 4 GiB, 8 GiB unpacked in all, paths
up to 160 characters. Paths must be relative and use `/`. They may not contain
`.`/`..` parts, `\ : * ? " < > |` or control characters, Windows device names
(`CON`, `NUL`, `COM1`…), or parts that end in a dot or a space. Two paths that
differ only in case are refused, because Windows and macOS would make them one
file.

## 3. Check and pack it

```sh
concat-cli check my-pack/effects          # the effect packages load, fixtures and probes pass
concat-cli extras pack my-pack            # writes me.starter-pack-1.0.0.concatpkg beside the folder
concat-cli extras inspect me.starter-pack-1.0.0.concatpkg   # every member checked as an install would
```

A `.concatpkg` is a gzip-compressed tar. Its first member is `extra.toml`
(with the file list), followed by the listed files in path order. It contains
no timestamps or owners, so packing the same folder twice gives the same bytes.
Any tar tool can open it.

A single `.concatpkg` can be installed with **Extras → Install from file…**.
Unless a repository's signed index lists that exact file, Concat says it is
unverified and asks first.

## 4. Publish a repository

```sh
concat-cli extras keygen repo-secret.key                       # once; keep the secret private
concat-cli extras repo ./out --key repo-secret.key --name "My extras" --valid-days 90 my-pack other-pack
concat-cli extras verify ./out/index.json --public-key <hex>
```

`out/` then holds the archives, `index.json` and `index.json.sig`. The
signature is Ed25519 over the exact bytes of `index.json`, in hex. Serve the
folder over HTTPS, or use it from disk. Archive URLs in the index are relative,
so the folder can move.

### The index

```json
{
  "schema": 1,
  "repository": "My extras",
  "revision": "1731585600",
  "serial": 1731585600,
  "expires": 1739361600,
  "extras": [
    {
      "id": "me.starter-pack", "name": "Starter Pack", "version": "1.0.0",
      "summary": "…", "description": "…", "author": "Me", "licence": "CC0-1.0",
      "homepage": "", "category": "effects", "min_app": "0.2.11", "platforms": [],
      "keywords": ["glow"], "provides": ["effects"],
      "provides_ids": ["me.warm-glow", "me.diagonal-wipe"],
      "requires": [], "installed_size": 1830, "featured": false, "changelog": "",
      "download": { "url": "me.starter-pack-1.0.0.concatpkg", "size": 1204, "sha256": "…" }
    }
  ]
}
```

`featured` and `changelog` can be edited by hand before signing. Re-sign after
any edit, or Concat refuses the index. A download URL is either `https://…` or
a path relative to the index.

### Using a repository in Concat

The official repository is compiled in (`concat_host::extras::OFFICIAL_INDEX`
and `OFFICIAL_KEY`). Any other repository goes in
`<config>/extras/repositories.toml` (on Windows,
`%APPDATA%\app.concat.editor\extras\repositories.toml`):

```toml
[[repository]]
name = "My extras"
index = "https://extras.example.com/v1/index.json"   # or file:///D:/repo/out/index.json
key = "…the public key from keygen, 64 hex digits…"
```

A repository with no valid key, or with a plain `http://` address, is
reported and not used.

`serial` must go up with every publish: Concat refuses an index older than
one it has already seen from that repository. `expires` (Unix seconds, 0 for
never) is when Concat stops believing the index. Re-publish before then.
`concat-cli extras repo` sets the serial to the current time and the expiry to
`--valid-days` (365 by default) from now. Both are signed with everything else.

## Where installed content appears

| Kind | In Concat |
|---|---|
| effects | The Effects, Filters, Transitions and Audio-effects shelves and the inspector. Also in `concat-cli api`/`serve`/`preview`, MCP, and the API's `catalogue.list`. |
| fonts | The font picker, for every title |
| title presets | Text & Captions → Styles, after the built-in ones |
| stickers | Stickers → **Pictures**. A press places the picture at the playhead, copied into the project's `stickers/` folder |
| sounds | Audio → **From Extras**: preview, **+**, or drag, copied into the project's `audio/` folder |

The API's `extras.list` reports what is installed. A project keeps its own
copies of placed stickers and sounds, so removing the extra doesn't break it.
An effect or transition from an extra that is removed or switched off stays
linked in the project. Concat names the extra when the project opens, and the
effect shows again once the extra is back.

## Where installed extras go

```
<data>/extras/                      (%LOCALAPPDATA%\app.concat.editor\extras on Windows,
                                     the portable folder in a portable build)
  installed.json                    what is installed: the only source of truth
  packages/<id>/                    one installed extra: extra.toml and its files
  downloads/                        archives on their way in (.part while downloading)
  staging/  trash/                  in-flight installs and removals, swept at start
  cache/<repository>/               each repository's last good, signed index
```

Concat's own user folders (`config/effects`, `config/fonts`,
`config/text-presets`) are never touched. Extras sit beside them, and the
person's own content wins when an id repeats.

## Versions, updates and dependencies

* An installed extra with a newer version in a repository shows under
  **Updates**. Updating replaces the old version atomically.
* Concat never downgrades. Installing the same version again repairs a
  damaged install.
* `requires` is resolved before anything downloads. Dependencies install
  first. A missing dependency, a version range nothing satisfies, an upgrade
  that would break another installed extra, or a cycle all stop the plan with
  the reason.
* An extra another enabled extra needs cannot be removed or switched off
  until that one is.
