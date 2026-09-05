# Translating Concat

Concat speaks the language you choose in Settings › General › Language, and a
language is one file. This is how to add or improve one.

## How it works

Every string a person reads in the interface is looked up by its English
text. A locale is a JSON file that maps that English to your language:

```json
{
  "_": { "name": "Deutsch" },
  "Settings": "Einstellungen",
  "Imported {0} files": "{0} Dateien importiert",
  "Split at playhead (S, or ⌘B for every clip)": "Am Abspielkopf teilen (S, oder ⌘B für jeden Clip)"
}
```

- The `_` entry names the language in its own words. That name is what the
  Language list shows, so someone who cannot read the current language can
  still find their own.
- Every other key is the English exactly as it appears in
  [`engine/crates/concat/locales/en.json`](engine/crates/concat/locales/en.json),
  the complete inventory. Copy that file, keep the keys, replace the values.
- `{0}`, `{1}` and so on are filled in at run time — a count, a name, a
  file size. Keep them, and put them where your language wants them.
- A key your file leaves out reads in English. Nothing breaks; the line is
  simply not translated yet.

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

1. Put the file in `engine/crates/concat/locales/`.
2. Add its code and file to the `BUILT_IN` table at the top of
   `engine/crates/concat/src/i18n.rs`.
3. Run `python3 scripts/locales.py --check`. It lists every line each
   locale still lacks, and refuses a key nothing in the source asks for.
4. Open a pull request. Corrections to the languages Concat already ships
   are just as welcome as new ones.

Concat ships English, Deutsch, Español, Français, Italiano, 日本語, 한국어,
Português (Brasil), Русский, Türkçe and 简体中文.

## For developers

New strings go through the same lookup: `I18n.t("...")` in the `.slint`
tree, `t("...")` or `tf("...", &[...])` in the window's Rust, with the
English as the key. Names in effect manifests and text presets are looked
up the same way and need no wrapping. After adding strings, run
`python3 scripts/locales.py` to bring `en.json` up to date; CI runs the
check.
