# Typography controls

The Text inspector's Typography section uses the existing `IconButton`
primitive for both rows. Its `checked` property owns the selected appearance;
the inspector adds no colours, borders or custom pressed-state rules. These
buttons suppress hover tooltips, while retaining their labels.

- **B** toggles the existing font weight between bold (700) and regular (400).
  Weights of 700 or more show the button selected. The weight selector remains
  available and reflects the same value.
- **I**, underlined **U**, and struck **S** independently toggle italic,
  underline and strikethrough. All four styles can be combined.
- **AA**, **aa**, and **Aa** select uppercase, lowercase and sentence case.
  Clicking the selected case again restores the original appearance.
  Sentence case lowercases the text and capitalizes its first alphabetic
  character and those after `.`, `!`, `?` and their full-width equivalents.
  This is punctuation-based casing, without language or abbreviation detection.

Case changes preserve `content`, including its original spelling and case.
The host converts a copy before font selection, shaping and wrapping, for both
live preview and video export. Plain-text exports retain the source content.
Decorations use the selected face's underline/strike metrics with an em-based
fallback and join the glyph path, sharing fill, outline and shadow.

The new `textCase`, `underline` and `strikethrough` style fields are saved with
the project and reusable styles. Old styles default to original case with no
decorations. Inspector edits use the existing batch-edit and undo/redo path.

This branch starts at upstream commit
`797c8c707be5f2c798bb99e0fffad20612552510` and does not include the separate
caption features or personal Windows workflow changes.

Validation: source wiring, JSON parsing and whitespace checks were performed.
Rust regression tests cover source preservation, style serialization, old-style
defaults and decoration pixels/layout. They have not been run: the Rust
toolchain is unavailable locally and no build was requested. Manual UI and
video-export verification remain pending. New strings are supplied in English and Italian, with explicit English
fallback entries for the other locales. The locale inventory check passes.
