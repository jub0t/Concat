# Translating Concat

Concat's interface speaks whatever your system speaks, when a translation
exists. Adding one takes a text editor and nothing else — no build tools, no
code beyond two registry lines.

## What is translated

The desktop app's interface: menus, panels, dialogs, tooltips, toasts. Not
translated (yet): error text coming from the engine, and the website.

## Adding a language

1. Copy `desktop/src/locales/en.json` to `desktop/src/locales/<tag>.json`,
   where `<tag>` is the [BCP 47](https://en.wikipedia.org/wiki/IETF_language_tag)
   tag for your language — `zh-CN` for Simplified Chinese, `de` for German.
2. Translate the **values** only. Keys stay exactly as they are.
3. Register the language in `desktop/src/lib/i18n.ts`: one entry in `LOCALES`
   (use the language's **native** name — `简体中文`, not `Chinese`) and one
   line in `CATALOGS`.
4. Check your work: `cd desktop && npm install && npm test`. The i18n test
   names every key you missed, every stale key, and every placeholder that
   doesn't match.

Then open a pull request. That's the whole process.

## Rules that keep translations working

- **Translate whole sentences.** Word order differs between languages, which
  is why no message is ever assembled from fragments.
- **Keep `{placeholders}` exactly as written.** You can move them anywhere in
  the sentence, but the name inside the braces must survive verbatim —
  `"Save failed: {message}"` → `"保存失败：{message}"`.
- **Fill both plural forms.** Keys ending in `.one` and `.other` are plural
  pairs (`{count}` is available in both). If your language doesn't
  distinguish — Chinese doesn't — put the same text in both. The right form
  is picked by your language's own rules, not by English's.
- **Leave alone:** font names, technical units (dB, st, GB, fps), and
  keyboard shortcut hints, unless your platform's convention truly differs.

## Keeping a translation current

When new interface text lands, `en.json` gains keys and your file is
temporarily behind — the app falls back to English for those strings, and
`npm test` lists exactly which keys your file lacks. Update the file, run the
test, open a pull request.
