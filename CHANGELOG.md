# Changelog

One entry per release, newest first. Plain lists of what changed for the
person using the app; internal refactors appear only when they change
behaviour.

## Unreleased

## v0.2.0-beta.1 — 2026-09-03

The first beta: the editing surface CapCut users expect, on an engine that
renders effects on the GPU.

- Titles render. A text clip is painted into the frame in the monitor and
  the export, with its font, outline, shadow, plate and spacing; captions
  are visible for the same reason.
- Select, move, scale and turn pictures on the monitor, with a lime box and
  grips, snapping to the frame and to other pictures with dashed guides.
- The inspector is three tabs with sections: Video (Basic, Speed,
  Animation, Adjust), Audio (Sound, and Speed on a sound clip) and Effects
  (Filters, Effects). Adjust is a colour panel: exposure, brightness,
  contrast, saturation, temperature, tint, shadows, highlights, sharpen,
  vignette and fade.
- Effects are packages the inspector reads: every knob a package declares
  is a control, and every filter has an intensity. Applying a card from
  the library opens the inspector on it.
- Filters as layers: drag a look onto the lanes and it treats everything
  beneath it for as long as it runs, with a strength and ramps.
- Speed curves (Montage, Hero, Bullet, Jump Cut, Flash In, Flash Out) and
  Reverse, for picture and sound alike.
- Animation: In, Out and Combo shapes with a length, on video, stills and
  titles.
- Flip, blend modes (Multiply, Screen, Add, Lighten, Darken) and crop.
- New effects: Green Screen and Blue Screen, three masks, and for sound
  Normalize Loudness, Noise Reduction, Enhance Voice, Pitch, Lo-fi and
  Distorted; Bright, Radio, Hall, Plate and Cathedral render at last.
- Every visual effect is a shader on the GPU, with the FFmpeg chain kept
  for a machine without one.
- Undo and redo answer Cmd+Z and Shift+Cmd+Z, and a dragged control is
  one undo step.
- Transitions and audio effects show as compact named cards.
- The engine links FFmpeg and compiles whisper.cpp in: a build is one
  binary with no tools beside it. Seeks are frame-accurate and every
  decoded frame carries its real timestamp.
- Releases are built for six targets - macOS on Apple silicon and Intel,
  Linux and Windows on x86_64 and arm64 - as self-contained bundles, and
  every push to main refreshes a nightly.
- The editor window is built in Slint as the `concat` crate inside the
  engine workspace: one native Rust binary, embedding its own fonts and
  effect previews, driving the engine in-process.
- Nix flake for Linux: `nix build`, `nix run` and `nix develop` at the
  repository root, with FFmpeg, whisper.cpp and the speech libraries pinned.
- Text to speech: File → Text to speech turns typed narration into an audio
  clip at the playhead, spoken by one of 36 Kokoro voices (American and
  British English, Chinese) at a chosen pace. Generation runs entirely on
  this machine; the voice model downloads once (about 130 MB) from the sheet
  itself or Settings → Speech.

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
