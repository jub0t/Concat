# Changelog

One entry per release, newest first. Plain lists of what changed for the
person using the app; internal refactors appear only when they change
behaviour.

## Unreleased

- The engine links FFmpeg and compiles whisper.cpp in: nothing spawns an
  `ffmpeg`, `ffprobe` or `whisper-cli` process any more, so a build is one
  binary with no tools to stage beside it. Seeks are frame-accurate and
  every decoded frame carries its real timestamp.
- Releases are archives now, built by the `release.yml` workflow from a
  pushed `v*` tag: one binary per platform, with FFmpeg's runtime libraries
  beside it on Windows and Linux. Installers return with packaging.
- The editor window is rebuilt in Slint as the `concat` crate inside the
  engine workspace: one native Rust binary with no web view, embedding its
  own fonts and effect previews, driving the engine in-process. The Tauri +
  React app is gone, and with it Node, npm and the Nix flake.
- Text to speech: File → Text to speech turns typed narration into an audio
  clip at the playhead, spoken by one of 36 Kokoro voices (American and
  British English, Chinese) at a chosen pace. Generation runs entirely on
  this machine; the voice model downloads once (about 130 MB) from the sheet
  itself or Settings → Speech.
- The entire interface is translatable: the app follows the system language
  when a translation exists, with an override in Settings → General.
  Translations are plain JSON files contributors can add — see TRANSLATING.md.
- Simplified Chinese ships as the first translation (machine-drafted,
  pending native review).

## v0.2.0-alpha.6 — 2026-08-29

- Effect tiles preview the real render: each thumbnail comes from the
  effect's actual FFmpeg chain, not an approximation.
- Editor icons come from Lucide.

## v0.2.0-alpha.5 — 2026-08-29

- Timeline tabs reorder by dragging, following the pointer live.
- Toasts rise in, hold long enough to be read, and sink out.
- Tab drops land where the preview caret shows.
- Transcriber settings select models by card; the internal engine row is gone.
- Color fixes: panel resizers, the idle play button and dark sunken surfaces
  now sit correctly in the palette.

## v0.2.0-alpha.4 — 2026-08-29

- Nix flake for Linux.
- Portrait phone video imports as portrait: probing reports displayed
  dimensions.
- A project closed before its first edit reopens empty instead of corrupt.
- Edits dispatched before the session opens wait for it.
- Packagers that guarantee PATH tools can skip the bundle guard.

## v0.2.0-alpha.3 — 2026-08-29

- Preview quality picker replaces the footer fps readout.
- Fixed same-tick edits reaching the engine empty.

## v0.2.0-alpha.2 — 2026-08-29

- Clock-paced playback stream with engine decode-ahead.
- Waveform peaks decode in the engine.

## v0.2.0-alpha.1 — 2026-08-29

- The engine owns the whole render path: export and preview render the
  engine's session, and the FFmpeg chains are built engine-side.
- Stacked glow/mirror effects no longer break the export.
- A lost GPU device degrades to the CPU compositor instead of failing.
- Playback no longer re-renders the whole interface at 60fps.
- Timecode, trim and autosave fixes in the timeline.
- File access starts empty and grows only by user intent.
- Every green main build publishes the next alpha automatically.

## v0.1.0-alpha.1 — 2026-08-27

- First public alpha: timeline editing, media bin, effects and filters,
  text and captions, export via FFmpeg, on-device transcription.
