# Translating Concat

Concat speaks the language you choose in Settings › General › Language, and a
language is one file. This is how to add or improve one.

## How it works

Every string a person reads in the interface is looked up by a key. A
locale is a JSON file that maps each key to your language:

```json
{
  "_": { "name": "Deutsch" },
  "common.settings": "Einstellungen",
  "mediaBin.importedFiles": "{0} Dateien importiert",
  "tray.splitAtPlayhead": "Am Abspielkopf teilen (B, oder ⌘B für jeden Clip)"
}
```

- The `_` entry names the language in its own words. That name is what the
  Language list shows, so someone who cannot read the current language can
  still find their own.
- Every other key is one of those in
  [`src/crates/concat/locales/en.json`](src/crates/concat/locales/en.json),
  the complete inventory, which holds the English for each. Copy that file,
  keep the keys, replace the values.
- A key is dotted lowerCamelCase: the part of the interface the string
  belongs to (`common` for one used all over), then a name for it. Effects
  are under `effects.` - `effects.goldenHour.name`, `effects.labels.amount`
  - and text presets under `presets.`.
- `{0}`, `{1}` and so on are filled in at run time — a count, a name, a
  file size. Keep them, and put them where your language wants them.
- A key your file leaves out reads in English. Nothing breaks; the line is
  simply not translated yet.
- A file from before keys, keyed by the English itself, still loads: each
  line is read as the key `en.json` gives that English.

The file's name is the language code: `de.json`, `pt-BR.json`, `zh-Hans.json`.

## Trying a translation without building

Drop your file into the `locales` folder of Concat's config directory and
restart the app; the language appears in Settings.

| Platform | Folder |
|---|---|
| macOS | `~/Library/Application Support/app.concat.editor/locales/` |
| Linux | `~/.config/app.concat.editor/locales/` |
| Windows | `%APPDATA%\app.concat.editor\locales\` |

A file there with the code of a language Concat ships lays its lines over
the shipped ones, so a correction is a file holding only the lines that
change.

## Shipping a language with Concat

1. Put the file in `src/crates/concat/locales/`.
2. Add its code and file to the `BUILT_IN` table at the top of
   `src/crates/concat/src/i18n.rs`.
3. Run `python3 scripts/locales.py --check`. It lists every line each
   locale still lacks, and refuses a key nothing in the source asks for.
4. Open a pull request. Corrections to the languages Concat already ships
   are just as welcome as new ones.

Concat ships English, Deutsch, Español, فارسی, Français, Hrvatski, Italiano,
日本語, 한국어, Português (Brasil), Русский, Türkçe, 简体中文 and 正體中文.

## For developers

New strings go through the same lookup by key: `I18n.t("area.name")` (or
`t1`, `t2`, `upper`) in the `.slint` tree, `t("area.name")` or
`tf("area.name", &[...])` in the window's Rust. Name the area after the file
the string lives in - `mediaPane`, `export`, `studio` - or `common` for a
string used in several, and the string after what it says: `export.tenBitColour`.
Write its English into `en.json` yourself; the source only holds the key.

Names in effect manifests and text presets need no key in the source: they
are looked up by keys made from the package's or preset's id, and
`python3 scripts/locales.py` writes their English into `en.json` from the
manifests. Run it after adding strings; `--check` fails on a key with no
English, English written where a key belongs, or a stale line.
