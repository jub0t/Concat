# AlMosawiCut

A native iOS video editor (SwiftUI + AVFoundation + Core Image), built from the
feature set of [Concat](https://github.com/jub0t/Concat), the open-source
CapCut alternative. No code, assets or branding from Concat are used: Concat is
a Rust/Slint desktop app under AGPL-3.0 with a trademarked name, so this is a
fresh implementation of the same ideas for iPhone and iPad.

## Features

- Projects saved on device (Documents/Projects): duplicate, rename, delete
- Import videos and photos from Photos (photos become clips)
- Scroll-to-scrub timeline with filmstrips, pinch to zoom, separate overlay, text and audio lanes
- Edit: split, trim, speed (0.1x-10x) and 6 speed curves, volume/mute, freeze frame,
  reorder, duplicate, delete, fit/fill, mirror, flip, crop with ratio presets
- Keyframes: scale, position, rotation, opacity and volume, plus quick motions
  (zoom, pan, Ken Burns, spin in, slide up); drag, pinch and rotate on the preview
- Overlays (picture-in-picture): 12 blend modes, 5 mask shapes with feather, own trim/speed/effects
- 56 filter looks in 11 categories with intensity, 10 adjustments, 32 video effects
- 25 transitions with duration, apply to all cuts
- Remove background: people or any subject (Vision, on device), green/blue screen, luma key, remove black;
  canvas background colours or blur
- Text: 5 styles, 17 built-in fonts including Arabic (Naskh, Geeza, Damascus, Nastaliq), plus your own
  .ttf/.otf/.ttc fonts imported from Files (kept in the app for every project), 7 in/out animations,
  10 templates, 8 shapes
- Auto-captions: Apple speech recognition (Arabic, English and more), on device when the
  language model is installed; restyle every caption at once
- Audio: your own music or sounds from Files (songs or videos), the sound of a video in Photos,
  voiceover recording with the mic while the video plays, loop, trim, fades, text to speech with device voices and Personal Voice,
  extract audio from a clip, voice cleanup (enhance, noise reduction, normalise), 22 voice effects
- Canvas 9:16, 16:9, 1:1, 4:5, 4:3, 3:4, 21:9
- Export 720p/1080p/2K/4K at 24/30/60 fps, H.264 or HEVC, save to Photos or share. No watermark.
- Undo/redo for every edit

## Build

The checked-in `AlMosawiCut.xcodeproj` is ready to open. To regenerate it from `project.yml`:

```bash
brew install xcodegen
xcodegen generate
```

```bash
open AlMosawiCut.xcodeproj
```

To run on your iPhone: plug it in (or pair it over Wi-Fi), pick it as the run destination,
choose your team under Signing & Capabilities (a free Apple ID works), and press Run. On the phone,
turn on Settings > Privacy & Security > Developer Mode the first time, and trust your developer
profile under Settings > General > VPN & Device Management.

From Xcode, launching with the `-open` argument (Debug builds) jumps straight into the newest project.

The `AlMosawiCut iOS` GitHub workflow (`.github/workflows/ios-app.yml`) builds for the Simulator and
for iPhone on every push that touches the app, opens a test project in the Simulator and uploads
screenshots.

## How it fits together

- `Model/` - `Project` (clips, texts, music) as plain Codable values; `ProjectStore` persists them.
- `Engine/CompositionBuilder` - lays the project out as an `AVMutableComposition` + audio mix.
- `Engine/FrameRenderer` - one function that turns a source frame into a finished canvas
  frame (orient, adjust, filter, fit, fade, titles). Both preview and export use it.
- `Engine/Compositor` - an `AVVideoCompositing` that runs FrameRenderer for export.
- `Views/EditorView` `FrameView` - the preview pulls raw frames from the player and runs
  FrameRenderer itself, so look changes are instant and nothing depends on live
  `AVVideoComposition` playback (which fails on the iOS 27 Simulator).
