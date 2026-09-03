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

## Slint — used under GPL-3.0-only

The `concat` crate (`engine/crates/concat`) builds against
[Slint](https://github.com/slint-ui/slint), which its authors offer under
**any one** of three licences, at the user's choice: a Royalty-free licence, a
paid commercial licence, or **GNU GPL-3.0-only**.

**Concat uses Slint under the GPL-3.0-only option.** That choice is deliberate
and it is recorded here because nothing in the source tree would otherwise say
which of the three applies. The Royalty-free and commercial options are not
used: both are aimed at shipping proprietary applications, and neither can
grant downstream recipients the freedoms Concat's own licence promises them.

Section 13 of the GPL-3.0 exists for exactly this combination:

> Notwithstanding any other provision of this License, you have permission to
> link or combine any covered work with a work licensed under version 3 of the
> GNU Affero General Public License into a single combined work, and to convey
> the resulting work.

So the combined binary is conveyable. Slint's portion remains GPL-3.0-only,
Concat's portion remains AGPL-3.0-or-later, and the AGPL's section 13 network
requirement applies to the combination as a whole. Anyone forking Concat who
would rather not be bound by the GPL must take Slint under one of its other
two licences and remove or replace Concat's AGPL-licensed code accordingly;
the two cannot be mixed.

Slint pulls in the renderer selected by the feature flags in
`engine/crates/concat/Cargo.toml` — Skia by default, FemtoVG over wgpu under
`--features wgpu` — along with winit and their transitive crates, which are
predominantly MIT/Apache-2.0/BSD licensed. `cargo tree -p concat` gives the
resolved set of any given build.

## Fonts

Cabinet Grotesk and Synonym are bundled under the ITF Free Font License; the
full texts ship beside the font files in `desktop/src/assets/fonts/`.

The Slint window embeds its fonts into the binary
(`engine/crates/concat/build.rs`, `EmbedResourcesKind::EmbedFiles`), so a
distributed binary carries them and their licences travel with it. Full texts
are in `engine/crates/concat/ui/fonts/`.

- **Inter** — SIL Open Font License 1.1. Copyright (c) 2016 The Inter Project
  Authors, https://github.com/rsms/inter. See `ui/fonts/LICENSE.txt`.
- **Synonym** — ITF Free Font License 2.0, Indian Type Foundry, distributed
  via https://www.fontshare.com. See `ui/fonts/LICENSE-Synonym.txt`.

Neither licence permits selling the fonts on their own, and the OFL requires
that Inter's copyright notice and licence travel with any redistribution. Both
are satisfied by shipping the `fonts/` directory as it stands.

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
`scripts/generate-effect-previews.mjs`. The Slint window embeds its own copy
of the tiles from `engine/crates/concat/ui/assets/effect-previews/`.
