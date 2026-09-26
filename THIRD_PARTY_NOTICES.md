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
originate at https://huggingface.co/ggerganov/whisper.cpp (MIT); Concat
mirrors them and downloads them on demand from that mirror, and never
bundles them. See "The model mirror" below.

## Slint — used under GPL-3.0-only

The `concat` crate (`src/crates/concat`) builds against
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

Slint pulls in its Skia renderer (see `src/crates/concat/Cargo.toml`) along
with winit and their transitive crates, which are
predominantly MIT/Apache-2.0/BSD licensed. `cargo tree -p concat` gives the
resolved set of any given build.

## Fonts

The window and the title painter embed the interface's font into the binary
(`src/crates/concat/build.rs`, `EmbedResourcesKind::EmbedFiles`, and
`include_bytes!` in `concat-text`), so a distributed binary carries it and its
licence travels with it. The files and the full licence text are in
`src/crates/concat-text/fonts/`.

- **Hanken Grotesk** — Copyright 2021 The Hanken Grotesk Project Authors
  (https://github.com/marcologous/hanken-grotesk), SIL Open Font License
  1.1. The Regular, Medium, SemiBold, Bold and Italic static instances are
  embedded. See `fonts/LICENSE-HankenGrotesk.txt`.

The licence does not permit selling the font on its own; shipping the
`fonts/` directory as it stands satisfies it.

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
https://huggingface.co/hexgrad/Kokoro-82M) originate in the sherpa-onnx
releases - including espeak-ng's data files. Concat mirrors those bundles
and downloads them on demand from that mirror, and never bundles them with
the app. See "The model mirror" below.

The Pocket TTS bundle is Kyutai's Pocket TTS (CC-BY-4.0,
https://huggingface.co/kyutai/pocket-tts, https://kyutai.org) as exported
to ONNX by KevinAHM (https://huggingface.co/KevinAHM/pocket-tts-onnx,
CC-BY-4.0) and packaged in the sherpa-onnx releases, with two sample
recordings from Kyutai's delayed-streams-modeling repository as its named
voices. It is downloaded on demand like the Kokoro bundles. Reading in the
voice of a recording is a use the person doing it is answerable for:
Concat offers it for the narrator's own voice, and Kyutai asks that no
one's voice be cloned without their consent.

Chatterbox Turbo (MIT, https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX,
Resemble AI) is driven through ONNX Runtime directly, from Resemble's own
ONNX export, downloaded on demand like the other voices. Its GPT-2
tokenizer is read by the tokenizers crate (Apache-2.0). Resemble's Python
package stamps a Perth watermark on what it speaks; the ONNX pipeline
carries no such step and Concat adds none, which MIT permits and which
anyone passing the sound off as a person's should know is not there.

## ONNX Runtime

The cutout models run on Microsoft's ONNX Runtime
(https://github.com/microsoft/onnxruntime, MIT), through the `ort` crate
(https://github.com/pykeio/ort, MIT or Apache-2.0). On macOS, Windows and
the phones it is linked into the app from pyke's builds of it; the Linux
bundles ship Microsoft's own shared build of the same version beside the
binary, in `lib/`, and its licence is in the release it was taken from
(https://github.com/microsoft/onnxruntime/releases).

## The cutout models

Remove background runs three models, none of which ship inside the app
except the first:

- Google's MediaPipe Selfie Segmentation (Apache-2.0), in the ONNX
  conversion published by the ONNX Community
  (https://huggingface.co/onnx-community/mediapipe_selfie_segmentation,
  Apache-2.0), compiled into the `concat-vision` crate; see
  `src/crates/concat-vision/models/NOTICE.md`. The answer when nothing
  has been downloaded.
- Robust Video Matting, the MobileNetV3 variant, by Peter Lin and others
  (https://github.com/PeterL1n/RobustVideoMatting, GPL-3.0), from that
  repository's releases, downloaded on first use. The person model.
- IS-Net from "Highly Accurate Dichotomous Image Segmentation" by Qin and
  others (https://github.com/xuebinqin/DIS, Apache-2.0), in the ONNX
  export the rembg project publishes
  (https://github.com/danielgatis/rembg, MIT), downloaded on first use.
  The object model.
- SlimSAM (https://github.com/czg1225/SlimSAM, Apache-2.0), in the ONNX
  export published at https://huggingface.co/Xenova/slimsam-77-uniform
  (Apache-2.0), downloaded on first use. The brushes' model.

Downloaded models live in the app's data directory under `cutout-models`
and are never bundled; the three that are downloaded come from Concat's own
mirror of the sources named above, described below. They are all run by
ONNX Runtime
(https://github.com/microsoft/onnxruntime, MIT) through the `ort` crate
(https://github.com/pykeio/ort, MIT OR Apache-2.0), with the platform's
own accelerator behind it: CoreML on macOS and iOS, DirectML on Windows,
NNAPI on Android. The runtime is linked statically from the builds pyke
publishes for each target.

## The model mirror

Every model named above is mirrored onto a release of Concat's own
repository, and the app fetches it from there; the upstream each was
obtained from is the fallback and is named above in every case. The table of
what is mirrored, and the digest each download is checked against, is
`models/manifest.toml`; `.github/workflows/models.yml` is what fills the
mirror.

Mirroring is redistribution, and each model keeps the licence it arrived
under - Apache-2.0 for the Kokoro bundles, SlimSAM and IS-Net, CC-BY-4.0
for Pocket TTS, BSD-3-Clause for Real-ESRGAN, MIT for Chatterbox Turbo and
the whisper conversions, GPL-3.0 for Robust Video Matting. Those terms are met
by the attributions above and by the licence files each mirrored archive
carries; a mirrored file is a verbatim copy, never a modification. Nothing
in the mirror is bundled with the app, and Concat claims no rights over any
of it.

## Effect preview photograph

The effect catalogue cards are drawn from a photograph by Vitaly Gariev on
Unsplash (https://unsplash.com/@silverkblack), used under the Unsplash
License. The still lives at `src/crates/concat-host/assets/card-still.jpg`
and is embedded in the app; each card is the effect's own shader run over
it on the GPU when the app first needs the card (`concat-host`'s
`cards.rs`).
