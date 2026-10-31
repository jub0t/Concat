# AI edit (Android)

A prompt box in the phone editor that plans an edit and applies it only
after you review it. It is zero-budget by construction: it calls only
free models, and there is no server in the middle — the phone calls the
provider you configured, directly.

## Using it

1. Open a project on a phone, and tap **AI edit** in the bottom bar
   (beside Captions and Speak).
2. Pick a provider:
   - **OpenRouter** — enter your OpenRouter API key. The base URL is
     fixed at `https://openrouter.ai/api/v1`.
   - **Custom** — enter a base URL and an API key for any
     OpenAI-compatible endpoint.
3. Tap **Fetch free models**. The app calls the provider's `/models`
   endpoint and lists only free models: an id ending in `:free`, or a
   model whose reported prompt and completion prices are both zero.
   A provider that reports no free models gives an empty list — it
   never falls back to paid models. Pick one.
4. Type what should change, for example "cut the first clip in half,
   then add the title Summer at 2 seconds". Paste reference links if
   you want (comma-separated); they are style and structure references
   for the model, not downloads.
5. Tap **Generate plan**. The plan appears in plain language.
6. Tap **Apply this plan** to apply it, or **Discard plan** to drop it.

Photos and videos reach the timeline through the Media sheet, the way
any import does. The model is told the bin's media (id and name) and
the timeline's clips (id, name, start, duration), so a plan can lay
bin items out in order — but it cannot invent a file, an effect, or an
id, and it never sees your footage.

## What the model can and cannot do

The model does not edit video directly. It returns JSON — a summary and
a list of steps — and each step is one Concat edit command. The plan is
validated before it is applied:

- Only these ops are allowed: `addClipsInSequence`,
  `addClipAtFirstFree`, `addTextClip`, `addLayerClip`, `splitClips`,
  `trimClip`, `removeClips`, `moveClips`, `setClipSpeed`.
- No nested batches, no media import, no timeline or project
  operations, no more than 50 steps, and times, speeds and ids must be
  in range.
- The validated steps become a single `Batch` command, so the whole
  plan commits atomically and undoes as one step. A plan that fails
  validation commits nothing.

That also bounds what it is good at: ordering clips, cutting,
trimming, titles, layers and speed, from the facts the app actually
knows. It is not vision: unless a free vision model is picked and
frames are sent to it (not in this version), the model plans from
names, durations and your words, not from watching the footage.

## Keys and privacy

Provider settings live in the window's private `settings.json`, on the
device, readable by the account alone. The key is sent only to the
provider you configured, and is never logged. The prompt, references
and plan are not sent anywhere else.

## Building

The feature lives in two places:

- `src/crates/concat-ai/` — the provider client, the free-model
  filter, the plan schema and its validation, with unit tests:
  `cargo test -p concat-ai`
- `src/crates/concat/src/ai.rs`, the phone sheet in
  `src/crates/concat/ui/phone/editor.slint`, and the `Editor` global
  properties in `src/crates/concat/ui/editor.slint` — the sheet, the
  settings, and the handlers that apply a plan through
  `Studio::apply`.

The Android APK is built the way the project already builds it (see
`.github/workflows/mobile.yml`): `cargo apk build --release -p
concat-android`, after `src/scripts/ffmpeg-mobile.sh` and
`src/scripts/sherpa-mobile.sh` for the target. A full Android build
needs the Android SDK/NDK and those vendor libraries; it is not a
phone-only job, and the desktop window (`cargo run -p concat`) is the
place to try the sheet first.

## Limits to know

- Free models are rate-limited and sometimes slow, truncated or
  wrong. A failed generation reports the provider's error; retrying
  with another free model is the intended recovery, not a retry loop
  that burns the rate limit.
- Reference links are not downloaded. Most link targets (social
  platforms, video sites) do not allow direct media download, so a
  link is context, not source footage.
- The sheet is in the phone editor only. The same window code runs on
  desktop, but the desktop has no AI panel yet.
