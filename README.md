# AlMosawiCut

A native iOS video editor (SwiftUI + AVFoundation + Core Image), built from the
feature set of [Concat](https://github.com/jub0t/Concat), the open-source
CapCut alternative. No code, assets or branding from Concat are used: Concat is
a Rust/Slint desktop app under AGPL-3.0 with a trademarked name, so this is a
fresh implementation of the same ideas for iPhone and iPad.

## Features

- Projects saved on device (Documents/Projects), duplicate, rename, delete
- Import videos and photos from Photos (photos become clips)
- Scroll-to-scrub timeline with filmstrips, pinch to zoom
- Split, trim, speed (0.1x-8x), volume/mute, freeze frame, reorder, duplicate, delete
- 20 filters with intensity, brightness/contrast/saturation, fade in/out, fit/fill, flip
- Titles: 4 styles, colours, size, timing, drag to position on the preview
- Auto-captions: Apple speech recognition (Arabic, English and more), on device when the
  language model is installed, grouped into short captions
- Background music from Files with volume, looping and an end fade
- Canvas 9:16, 16:9, 1:1, 4:5
- Export 720p/1080p/4K, H.264 or HEVC, save to Photos or share. No watermark.
- Undo/redo for every edit

## Build

```bash
brew install xcodegen
xcodegen generate
open AlMosawiCut.xcodeproj
```

Set your signing team in Xcode to run on a device.

## How it fits together

- `Model/` - `Project` (clips, texts, music) as plain Codable values; `ProjectStore` persists them.
- `Engine/CompositionBuilder` - lays the project out as an `AVMutableComposition` + audio mix.
- `Engine/FrameRenderer` - one function that turns a source frame into a finished canvas
  frame (orient, adjust, filter, fit, fade, titles). Both preview and export use it.
- `Engine/Compositor` - an `AVVideoCompositing` that runs FrameRenderer for export.
- `Views/EditorView` `FrameView` - the preview pulls raw frames from the player and runs
  FrameRenderer itself, so look changes are instant and nothing depends on live
  `AVVideoComposition` playback (which fails on the iOS 27 Simulator).
