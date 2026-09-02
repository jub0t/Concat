# Third-party notices

## FFmpeg

Concat bundles unmodified `ffmpeg` and `ffprobe` binaries and invokes them as
separate child processes; the app does not link against them.

- **Windows**: static GPL builds from the BtbN autobuild project
  (`ffmpeg-n8.1-latest-win64-gpl`), built with x264.
  Source and build scripts: https://github.com/BtbN/FFmpeg-Builds
- **macOS**: static builds by Martin Riedl.
  Source and build info: https://ffmpeg.martin-riedl.de

FFmpeg is licensed under the LGPL-2.1-or-later, with the bundled builds
compiled as GPL-2.0-or-later (they include x264). FFmpeg source code:
https://ffmpeg.org/download.html

## Fonts

Cabinet Grotesk and Synonym are bundled under the ITF Free Font License; the
full texts ship beside the font files in `desktop/src/assets/fonts/`.

## Whisper models (optional download)

Auto-captions can download ggml Whisper models from
https://huggingface.co/ggerganov/whisper.cpp (MIT). Models are fetched on
demand and never bundled.

## sherpa-onnx and Kokoro voices

Text to speech links the sherpa-onnx runtime statically
(https://github.com/k2-fsa/sherpa-onnx, Apache-2.0), which itself statically
links onnxruntime (MIT), piper-phonemize (MIT) and espeak-ng
(**GPL-3.0-or-later**, https://github.com/espeak-ng/espeak-ng) for
grapheme-to-phoneme conversion. Because espeak-ng is compiled into the app
binary, distributed builds must comply with the GPL-3.0 for that combined
work. Concat's own sources are AGPL-3.0-or-later; section 13 of both GPL-3.0
and AGPL-3.0 expressly permits that combination, so the combined binary may be
conveyed on those terms.

Kokoro voice model bundles (Apache-2.0,
https://huggingface.co/hexgrad/Kokoro-82M) are downloaded on demand from the
sherpa-onnx releases - including espeak-ng's data files - and are never
bundled with the app.

## Effect preview photograph

The effect catalogue thumbnails are rendered from a photograph by
Vitaly Gariev on Unsplash (https://unsplash.com/@silverkblack), used
under the Unsplash License. The source still lives at
`assets/effect-preview-source.jpg`; regenerate the tiles with
`scripts/generate-effect-previews.mjs`.
