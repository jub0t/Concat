# Third-party notices

## FFmpeg

Concat links FFmpeg's libraries - libavformat, libavcodec, libavfilter,
libswscale and libswresample - through the `ffmpeg-the-third` crate. The
Slint app spawns no `ffmpeg` or `ffprobe` process.

FFmpeg is licensed under the LGPL-2.1-or-later; builds that include x264
(which the H.264 export uses) are GPL-2.0-or-later. Concat's own sources are
AGPL-3.0-or-later, and section 13 of the GPL-3.0 and AGPL-3.0 expressly
permits linking the two, so a distributed build may be conveyed on those
terms. Which FFmpeg a binary carries depends on the machine that built it:
Homebrew's on macOS, a BtbN `shared` build (https://github.com/BtbN/FFmpeg-Builds)
on Windows and in CI. FFmpeg source code: https://ffmpeg.org/download.html

## whisper.cpp

Transcription compiles whisper.cpp (https://github.com/ggml-org/whisper.cpp,
MIT) and ggml into the app through the `whisper-rs` crate. Whisper models
are downloaded on demand from https://huggingface.co/ggerganov/whisper.cpp
(MIT) and never bundled.

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

The window embeds its fonts into the binary
(`engine/crates/concat/build.rs`, `EmbedResourcesKind::EmbedFiles`), so a
distributed binary carries them and their licences travel with it. Full texts
are in `engine/crates/concat/ui/fonts/`.

- **Helvetica Neue** — Copyright (c) 1981, 1997 Linotype-Hell AG. Neue
  Helvetica is a Monotype typeface, used under the licence held for it; the
  Roman, Medium and Bold faces are embedded.
- **Synonym** — ITF Free Font License 2.0, Indian Type Foundry, distributed
  via https://www.fontshare.com. See `ui/fonts/LICENSE-Synonym.txt`.

Neither licence permits selling the fonts on their own; shipping the `fonts/`
directory as it stands satisfies both.

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

## The cutout models

Remove background runs three models, none of which ship inside the app
except the first:

- Google's MediaPipe Selfie Segmentation (Apache-2.0), in the ONNX
  conversion published by the ONNX Community
  (https://huggingface.co/onnx-community/mediapipe_selfie_segmentation,
  Apache-2.0), compiled into the `concat-vision` crate; see
  `engine/crates/concat-vision/models/NOTICE.md`. The answer when nothing
  has been downloaded.
- Robust Video Matting, the MobileNetV3 variant, by Peter Lin and others
  (https://github.com/PeterL1n/RobustVideoMatting, GPL-3.0), downloaded on
  first use from that repository's releases. The person model.
- IS-Net from "Highly Accurate Dichotomous Image Segmentation" by Qin and
  others (https://github.com/xuebinqin/DIS, Apache-2.0), in the ONNX
  export the rembg project publishes
  (https://github.com/danielgatis/rembg, MIT), downloaded on first use.
  The object model.
- SlimSAM (https://github.com/czg1225/SlimSAM, Apache-2.0), in the ONNX
  export published at https://huggingface.co/Xenova/slimsam-77-uniform
  (Apache-2.0), downloaded on first use. The brushes' model.

Downloaded models live in the app's data directory under `cutout-models`
and are never bundled. They are all run by ONNX Runtime
(https://github.com/microsoft/onnxruntime, MIT) through the `ort` crate
(https://github.com/pykeio/ort, MIT OR Apache-2.0), with the platform's
own accelerator behind it: CoreML on macOS and iOS, DirectML on Windows,
NNAPI on Android. The runtime is linked statically from the builds pyke
publishes for each target.

## Effect preview photograph

The effect catalogue thumbnails are rendered from a photograph by
Vitaly Gariev on Unsplash (https://unsplash.com/@silverkblack), used
under the Unsplash License. The source still lives at
`assets/effect-preview-source.jpg`; the tiles are each effect's real FFmpeg
chain (`concat-export`'s `chains.rs`) run over it, and are embedded from
`engine/crates/concat/ui/assets/effect-previews/`.
