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
  "tray.splitAtPlayhead": "Am Abspielkopf teilen ({0})"
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
  belongs to (`common` for one used all over), then a name for it.
- Only the app's own interface is translated. Effects, filters,
  transitions and text presets are data, and always read in English, as
  their manifests write them: their names, descriptions and knob labels
  have no lines here.
- `{0}`, `{1}` and so on are filled in at run time: a count, a name, a
  file size, a shortcut. Keep them, and put them where your language wants them.
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
3. Give it a README: add `scripts/readme/lang_<code>.py` with the README's
   words in your language, add the code to `ORDER` in
   `scripts/readme/gen.py`, and run `python3 scripts/readme/gen.py`. The app
   and the READMEs speak the same languages, one for one.
4. Run `python3 scripts/locales.py --check`. It lists every line each
   locale still lacks, refuses a key nothing in the source asks for, and
   fails on a locale with no README or a README with no locale.
5. Open a pull request. Corrections to the languages Concat already ships
   are just as welcome as new ones.

Concat ships 25 languages: English, العربية, বাংলা, Deutsch, Español, فارسی,
Français, עברית, हिन्दी, Hrvatski, Bahasa Indonesia, Italiano, 日本語, 한국어,
Nederlands, Polski, Português (Brasil), Русский, ไทย, Türkçe, Українська,
اردو, Tiếng Việt, 简体中文 and 繁體中文.

## For developers

New strings go through the same lookup by key: `I18n.t("area.name")` (or
`t1`, `t2`, `upper`) in the `.slint` tree, `t("area.name")` or
`tf("area.name", &[...])` in the window's Rust. Name the area after the file
the string lives in - `mediaPane`, `export`, `studio` - or `common` for a
string used in several, and the string after what it says: `export.tenBitColour`.
Write its English into `en.json` yourself; the source only holds the key.

Names in effect manifests and text presets are never looked up, so add no
key for them. Run `python3 scripts/locales.py` after adding strings;
`--check` fails on a key with no English, English written where a key
belongs, or a stale line.
