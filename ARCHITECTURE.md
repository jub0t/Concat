# Concat: architecture

Concat is a video editor. It is one native Rust binary: a headless engine
that owns the edit, the pixels and the sound, and a Slint window that draws
that engine and hands it commands. FFmpeg is linked, whisper.cpp and
sherpa-onnx are compiled in, and nothing is spawned at run time.

This document is the map. It says what each part is for, which rules hold
across parts, and where the work goes next. Every crate's `lib.rs` opens
with a `//!` block that says the same thing at closer range; read this
first, then those.

---

## 1. The one rule

**The engine owns the edit. The window is a view of it.**

The project, its operations, its undo history, every pixel of every frame
and every sample of the mix are decided in the engine crates, which have no
window and no idea one exists. The window reads the engine's project to
draw the bin and the lanes, and writes every edit as a command. It keeps
what the document does not need: selection, playhead, zoom, tool, which
lanes are locked, the arrangement of panes. None of that reaches the file.

Two consequences of the rule shape everything below:

- **One definition per meaning.** The speed range, the arithmetic that maps
  a timeline instant to a source instant, the placement of a picture in the
  frame, the mix's gain curve: each exists once, and the preview, the
  export and the command layer all call it. Where two paths could disagree,
  the code names the one they share and says why.
- **Degrade, never abort.** A source that runs out contributes nothing; an
  unknown transition renders as a cut; a filter chain the catalogue refuses
  leaves the picture untouched; a GPU that dies mid-export hands the rest to
  the CPU; a document with vanished media opens without those clips. An
  export or a session never ends in a panic because a file was strange.

---

## 2. The map

Everything lives in `engine/`, one Cargo workspace of twelve crates.

| Crate | Owns | Lines |
|---|---|---|
| `concat-core` | Exact time, arenas and handles, the RGBA frame, the timeline model the renderer reads. No dependencies. | 2.2k |
| `concat-project` | The document: model, every operation as a command, undo, the tolerant reader and compatible writer. | 4.7k |
| `concat-effects` | Effect packages: manifests, the FFmpeg template language, WGSL shaders, the catalogue. 45 built-in packages. | 2.3k + packages |
| `concat-media` | FFmpeg, linked. Decoding, seeking, the audio mix planner, encoding, waveforms, the reader pool. | 3.7k |
| `concat-render` | A timeline plus an instant into one frame: the plan, the CPU compositor, the wgpu compositor. | 1.9k |
| `concat-export` | Timeline to file. Transitions resolve here; the frame loop and the one-pass mix; the paused monitor's true frame. | 2.2k |
| `concat-text` | Titles as pixels: font lookup, shaping, rasterising onto a frame-sized transparent canvas. | 0.6k |
| `concat-host` | What a window needs that is not the edit: sessions, project folders, caches, the monitor, audio playback, the export driver, templates, job slots, app directories. | 3.8k |
| `concat-speech` | Transcription with whisper.cpp and text to speech with Kokoro, both in-process, models downloaded on demand. | 1.2k |
| `concat-cli` | A driver for the engine without a window: `probe` and `render`. | 0.2k |
| `concat` | The editor window in Slint, and the Rust that binds it to the host. | 7.2k Rust, 15.9k Slint |
| `concat-android` | The Android activity: `android_main` over the window. | 20 |

The dependency arrows point one way:

```
concat ──► concat-speech ──► concat-host ──► concat-export ──► concat-render ──► concat-core
   │                              │               │    │            ▲
   │                              │               │    └──► concat-effects ──► concat-project ──► concat-core
   │                              ├──► concat-media ◄──┘
   │                              └──► concat-text
   └──► concat-effects (the catalogue, for the inspector's shelves)

concat-android ──► concat            concat-cli ──► concat-core, concat-media, concat-render
```

`concat-core` depends on nothing; `concat-project` is not folded into it
because the document needs serde and core's zero-dependency rule is worth
more than the adjacency. Five crates carry no native library and build for
the web as well (`concat-core`, `concat-project`, `concat-effects`,
`concat-render`, `concat-text`); CI keeps that true.

---

## 3. Vocabulary

The types every crate agrees on live in `concat-core`.

**Time is exact.** `time::Rational` is a reduced `i64/i64` in seconds.
Arithmetic widens to `i128` and reduces back; a value that comes from a
file goes through `checked_new`, which errors rather than panics, so a
hostile container cannot crash the engine. `approximate` is the one seam
through which an `f64` enters, at microsecond precision, and `as_f64` is
one-way out. `FrameRate` is exact too, and `frame_at` floors, so frames are
half-open intervals: 107,892 frames of 30000/1001 accumulate to exactly
where they should. Nothing in the engine keeps a timestamp as a float.

**Graphs use handles.** `arena::Arena<T>` hands out generational
`Id<T>` handles that are `Copy`, `Send` and `Sync` whatever `T` is. Removing
a slot bumps its generation, so a stale handle reads as empty rather than
as whatever now occupies the slot. The timeline's tracks and clips are two
arenas plus an explicit bottom-first `order`.

**One pixel format.** `frame::Frame` is RGBA8, straight alpha, sRGB,
tightly packed, with bounds-checked access that returns `None` and a
`Debug` that never dumps pixels. A higher-precision frame arrives as a
second type beside it, not as a `format` field.

**The timeline the renderer reads.** `timeline::Clip` carries start,
duration, source in-point, speed or a `SpeedCurve`, reverse, transform,
opacity, blend, fades and an `Animation`. `Clip::source_time_at` is the
one piece of arithmetic every trim, slip and retime has to agree with:
affine when the speed is constant, the curve's area when it is not,
mirrored when reversed. `is_paced` says whether a decoder can be pulled
one frame per output frame or must seek. `Blend` documents its six
premultiplied formulas so the CPU and GPU compositors agree by
construction. `retime::SpeedCurve` clamps to 0.0625..16 and yields the
constant-rate pieces a sound path that can only change tempo in steps
needs. `animate::Animation` is relative: scale multiplies, offsets and
rotation add, opacity multiplies.

**Two models, one boundary.** The document model in `concat-project` is
not the core timeline. It speaks the file's terms: `f64` seconds, `String`
ids, camelCase serde, the audio side that core's clip lacks (volume, fades,
pitch preservation, filter chains), text and layer clips that have no
media. The conversion from the document to `concat_core::Timeline` happens
in exactly one place, `concat-export`'s timeline builder, and the
`f64`-to-`Rational` step happens there. Two small converters inside
`concat-project` (`speed::curve_of`, `animation::animation_of`) are the
only other crossings.

---

## 4. The edit: `concat-project`

Two rules are stated at the top of the crate and hold throughout:

1. **The document format is frozen by the documents that exist.** Saved
   projects load forever, tolerance rules included. `DOCUMENT_VERSION` is
   bumped only when a change cannot be absorbed by defaulting.
2. **`f64` seconds and `String` ids are the document's terms**, until a
   version 2 decides otherwise on purpose.

**Commands.** `commands::Command` is a serialisable enum of about thirty
operations, tagged `op` in camelCase: add and remove media, place, move,
trim, split, merge and slip clips, set speed and speed curves, transforms,
animations, text, effects, track state, timelines, and `Batch`. The
constants the window's gestures mirror are declared once here: the
minimum clip duration (1/60 s), the speed range (the engine's, verbatim),
the maximum offset, the join tolerance. `ClipPatch` distinguishes absent
from null. Ids are minted by one counter across the `m`/`c`/`t`/`tl`
prefixes, and a loaded document is walked so restored ids are never
reissued. `Outcome::applied` is honest: a command that changes nothing
reports so, and the undo history records no phantom edits. `Batch` applies
to a staged clone and commits only when every step succeeds. `SplitClips`
drops a speed curve because the map to the source is no longer affine;
`MergeClips` defers to `why_not_merge`, which returns the exact sentence
the user sees.

**Undo.** `editor::Editor` keeps whole-project snapshots in a `VecDeque`
capped at 200; `apply` pushes only when the outcome was applied and clears
redo.

**Reading and writing.** `doc.rs` defaults every field, drops clips whose
track or media vanished (text and layer clips survive), clamps hand-edited
values into range, and loads a flat single-timeline document as one
timeline. The writer emits both `timelines` and a flat mirror of the
active one, so an older build still opens a new file.

**Named shapes.** Speed presets and animation presets are stored by name
and re-materialised per clip length; an animation slot longer than the
clip is squeezed, an overlapping in/out pair is split in proportion.

---

## 5. Effects: `concat-effects`

An effect is a folder. `effect.toml` declares an id of the form
`author.name`, its kind (audio, effect, filter, transition, generator), its
parameters with ranges and defaults, and one or two backends;
`fixtures.toml` pins what it renders at default, minimum and maximum. The
45 built-in packages under `packages/` are compiled into the binary by
`build.rs`, which scans the directory: adding an effect is adding a
folder, and nothing in Rust names one. User packages load from a directory
at run time.

**Two backends.** `[ffmpeg]` is a filter-chain template with
`{expression}` slots, run inside the decoder's filtergraph. The expression
language is a deliberately tiny recursive-descent parser over numbers and
eleven fixed functions; it is the security boundary as much as a
convenience, since nothing an expression evaluates to can be anything but
a number. `[wgsl]` is a shader that writes one `fn effect(uv) -> vec4<f32>`;
`shader.rs` wraps it in a prelude (the layer texture, a `Frame` uniform of
size, time and intensity, the package's `Params`) and a postlude (the
full-screen triangle and an `fs_main` that mixes the effect by intensity),
validates it through naga, and reads the `Params` struct's layout back from
the compiled module so the parameter bytes are packed exactly as the GPU
expects. A package may carry both: the shader renders wherever there is a
GPU, and the chain is what a machine without one gets.

**The catalogue.** `Catalogue::builtin()` loads once, panics on a bad
built-in, and rejects duplicate ids or aliases. Loading a package renders
its chain at default, minimum and maximum, so a type error in an expression
is a load failure and never a silent gap in an export. `video_chain`,
`video_chain_gpu` (which skips shader-backed packages), `shader_passes`
and `audio_chain` all funnel through one `compose`, which numbers labels
by emitted index so a bypassed entry consumes none. A *filter* (a colour
look) answers to an undeclared `intensity` key: `mixed` rewrites the chain
as a split-and-blend so intensity is the look's opacity.

The document is untouched by any of this. A clip stores
`{ id, params, enabled }`, and an id the catalogue does not know is skipped
at render time.

---

## 6. Media: `concat-media`

The only crate that knows FFmpeg exists. It links libavformat, libavcodec,
libavfilter, libswscale and libswresample through `ffmpeg-the-third`;
FFmpeg 7 is the floor because the display-rotation side data is read from
where 7.0 moved it. The crate's one `unsafe` block walks that side data.
`build.rs` names what the static archives on a phone need from the
platform (MediaCodec and the JNI on Android, VideoToolbox and its
frameworks on iOS); on the desktop the shared libraries bring their own.

**Two traits.** `FrameSource` (width, height, `next_frame`, `position`)
and `FrameSink` (`write_frame`, `finish`) fence everything codec-shaped. A
GPU-decode backend would implement those two and nothing else in the
workspace changes.

**Decoding.** `decode::Decoder` owns one input, one video decoder and one
filtergraph, no sharing and no interior mutability. The graph's order is
load-bearing: rotation, then the crop (in source pixels, since a crop
changes what the fit is *of*), then a bilinear scale to the requested
size, then the effect chain, then a guard scale back to that size, then
`format=rgba`. Every chain is validated before anything opens: a chain
containing `;`, `[`, `]` or a newline is no longer a filter applied to this
clip. Seeking is two-phase and frame-accurate: a container seek to the
target, then frames are discarded until the one whose timestamp reaches it.
`next_paced` keeps a frame around a target instant, duplicating and
dropping to hold a rate; `looping` repeats a still forever.

**The audio planner.** `audio.rs` plans a whole mix as one FFmpeg
filtergraph. Per clip: trim, speed (pitch-preserving `atempo` stages, or
the tape path of `asetrate` between resamples), the user's chain, volume,
fades, then `adelay` to its place on the timeline; then `amix` without
normalisation, or `anull` for one clip; then a resample and pad that fix
the timestamps `atempo` leaves undefined. A twenty-line comment explains
why that last stage exists and ends with the instruction not to simplify
it away. `mix_to_file` opens every input, feeds them round-robin and
drains after every push so no input runs ahead of the others by more than
a frame; `mux` stream-copies picture and sound together. The speed range
is defined here, once, and preview and export both read it.

**Encoding.** `encode::Encoder` defaults to `libx264`, medium, CRF 18,
`yuv420p` (4:4:4 will not play in browsers), writes with `+faststart`, and
keeps both the encoder's and the muxer's time bases for the rescale. A
frame of the wrong size is an error, not a resize. `jpeg` is the poster
encoder.

**Waveforms.** `peaks.rs` streams a mono 48 kHz decode into buckets
without accumulating samples, so an hour costs what a jingle costs, and
encodes a frozen little-endian cache format.

**The reader pool.** `pool::ReaderPool` is playback's infrastructure and
explicitly not export's: export decodes every frame once in order, and a
cache there is pure overhead. The pool is a byte-budgeted LRU of frames
keyed by file, instant, size and chain (the same source frame through two
chains is two pictures) over a bounded set of warm readers. Its ownership
rule: a reader is *found* under the pool's lock and *used* under its own,
so decodes on different files never queue behind each other. Asked for an
instant, it rolls forward up to 60 frames before it seeks, clamps a request
past the end to the last frame, and on failure retries seeks backwards by
widening steps, so a file with any decodable picture cannot fail here.

`treat.rs` is the decoder's filtergraph without the decoder: RGBA in,
chain, RGBA out, for a layer clip that has no pixels of its own.

---

## 7. Rendering: `concat-render`

Rendering splits in two, and the split is the crate.

1. **`plan`** answers what is on screen at this instant, from where, at
   what opacity, under what transform. It touches no pixels and does no IO,
   so it is fast, exactly testable, and identical for both backends. The
   fade ramp multiplies into the opacity here, so a compositor only ever
   sees a per-frame opacity and has no idea fades exist.
2. **`compositor`** takes the plan plus decoded pixels and blends them.

**The CPU compositor is the reference.** Placement is in pixels, converted
exactly once by whoever builds the layers. Aligned layers blend by region;
transformed layers are inverse-mapped with bilinear sampling over the
rotated bounding box, which is what makes the result hole-free at any
scale or angle. The blend table is the fixed-function set the GPU offers,
over premultiplied colour, so the two paths agree. Output is forced opaque.

**The GPU compositor exists to be fast and must match it.** `gpu.rs`
renders on wgpu into `Rgba8Unorm`, not sRGB, so blending happens on
gamma-encoded values exactly as on the CPU; when the day comes to blend in
linear light, both backends change together. Six pipelines, one per blend
mode, since blend state is baked into a pipeline. Layer textures are pooled
by size and retired after 300 idle composites. Effect shaders arrive as
`ShaderPass` data (a key, WGSL source, packed params, intensity); each is
compiled once per key and run as its own submission over ping-ponged
textures. Two outputs: `composite` reads back through a staging buffer,
and `composite_texture` leaves the result in a presentable ring for a
window that shares the device, which is how the monitor shows a frame
without a copy anywhere. Construction returns `None` without an adapter,
and a compositor that fails marks itself dead and hands every later frame
to the CPU.

**Parity is tested.** `assert_matches_cpu` composites the same layers on
both and asserts per-channel tolerance, at interior pixels for rotation
and scale where the two rasterisers legitimately differ on the half-pixel
border. The shader test compiles a real package through the catalogue's
stitching, so what the test runs is what a package runs. Every GPU test
passes silently on a machine with no adapter.

---

## 8. Export: `concat-export`

The seam where the document's flattened clip list becomes the engine's
timeline, and the engine decides everything after.

**Flatten.** `flatten_timeline` walks a document timeline into
`ExportClip`s: track index and media resolved, track state folded into
per-clip flags, filter chains built through the catalogue. Text clips are
left out here and rejoin as still images once painted (section 9). Layer
clips flatten to a pixel-less entry carrying their effects. Missing media
is a skip, not an error.

**Transitions resolve first.** `resolve_transitions` runs before anything
reads the clip list, so the picture and sound paths never know transitions
exist. Every track index is doubled so a cross-fade's incoming clip gets an
odd lane directly above the pair; the incoming clip is extended backwards
by the transition's duration, clamped to the outgoing duration, the
incoming duration and the available source handle, and given a video fade
in with the sound riding the picture. Fade-to-black and fade-to-white
become frame-based fade filters appended after the clip's own effects.
Adjacency tolerance is half a frame at the output rate. An unknown kind is
a cut.

**The frame loop.** One `Decoder` per paced clip, opened at its in-point at
`fps / speed`, so pulling exactly one frame per output frame keeps each in
step with the plan's source times without seeking. Unpaced clips (a speed
curve, reverse) go through a `ReaderPool`; stills repeat. Decoders retire
the frame their clip leaves the plan. `place_layer` is the one definition
the exporter and the preview share of where a picture goes in the frame.
Layer clips composite the stack beneath them, run their shader passes or
their FFmpeg treatment, and mix the result back by strength, ramped by
their fades. Progress reports every fifteen frames and at each stage, and a
cancel flag is checked at every frame. On the GPU, effects with a shader
are omitted from the FFmpeg chain and carried as passes instead.

**Sound is one pass.** Speed curves are cut into between 8 and 400
constant-rate pieces of about a tenth of a second, each at the curve's
local mean, with the clip's fades projected onto each piece; the pieces,
the paced clips and the unmuted video clips' own audio go to
`audio::mix_to_file` as a single filtergraph. Inputs with no audio stream
are filtered out by a probe first, because FFmpeg refuses a graph that
names a stream that is not there. Picture and sound are written to scratch
siblings beside the output and muxed at the end; the scratch is removed
whatever the outcome.

**The true frame.** `preview_frame` and `preview_sources` build the same
timeline the export builds, quantise the requested instant onto the frame
grid, and pull from the pool. `quantise` is the seam where a value stops
being approximate. Two invariants are pinned: fade-to-colour transitions
are not baked for the preview, because their frame numbers assume a decode
from the clip's start that pooled seeks break, and the window draws a
matching veil instead; and a plan with layers but nothing decoded is an
error rather than a black frame, because a caller would draw that "truth"
over its own perfectly good approximation. `PreviewSources` exposes bare
layers so a GPU caller can composite on the window's device.
`preview_prefetch` warms the pool one frame apart, which is exactly what
keeps its readers rolling forward instead of seeking.

---

## 9. Titles: `concat-text` and `host::titles`

A text clip is a style and some words; the compositor wants a picture.
`concat-text` finds the face (the system's fonts plus the project's own
files, falling back rather than failing), shapes each line with rustybuzz,
turns outlines into paths, and paints plate, shadow, outline and fill with
tiny-skia onto a canvas the size of the output frame, transparent
everywhere the words are not. Frame-sized on purpose: the compositor fits a
picture into the frame and applies the clip's transform about its centre,
so a canvas that *is* the frame fits at exactly one, decodes without
resampling, and puts the block's centre where the clip's centre is. A
title's offset and rotation then mean the same as footage's. Every size in
the style is a fraction of the frame's height, converted to pixels once at
the top, so a title looks the same at 720p and 4K.

`concat-host`'s `titles` module is the cache and the rejoin: each text
clip becomes an image `ExportClip` with the text clip's timing, transform,
keyframes and opacity, pointing at a PNG under the app's data directory
plus a JSON sidecar holding the block size the PNG cannot express. The file
is keyed by the style, the frame size, the project's fonts and a version
constant, so moving a title is free and only a change to the words or the
look repaints.

---

## 10. The host layer: `concat-host`

What the window needs that is not the edit. Nothing here knows about a
window; long work reports through callbacks and cancels through flags, and
the caller decides which thread it runs on. If a function here starts
deciding what an edit means, it belongs in `concat-project`.

**A project is a folder.** It holds `concat.json` (written at creation, so
a project is a real thing from the start), `cache/` (waveform peaks, the
poster, decoded audio for playback), `audio/` (narration the timeline
points at) and `assets/` (media a template brought). Saving writes a
sibling and renames it, so a truncated write loses nothing. Recents are a
machine-scoped list of twelve, filtered on read so an unplugged drive's
projects come back when it does.

**Session.** `Session` is a path, settings and an `Editor`. `apply`,
`undo` and `redo` return an `EditorView` (the project, undo and redo
availability, settings, the id a command created), so the window keeps no
model of its own. A document that fails to parse is refused unless the
manifest is settings-only, because silently replacing an edit with
emptiness is how projects get lost. The session flattens itself through
`concat-export`.

**Media.** Import probes a file into a `MediaItem` that stores enough
metadata to open meaningfully when the file is missing. Waveforms are 200
buckets a second, cached beside the project under an FNV-1a key that is
pinned by a test, because a changed hash would orphan every project's
caches. Filmstrips are up to 60 slices blitted into one wide frame, one
texture upload instead of many. The poster is regenerated only when the
manifest is newer than it.

**The monitor.** `Monitor` holds the app's one `ReaderPool` and, when the
window created one, the shared wgpu compositor. With a GPU, the paused
frame's layers go straight to `composite_texture` and no pixel comes back
down, unless a layer clip is live at that instant, in which case the CPU
composites and one frame is uploaded. Prefetch is capped at eight frames so
a confused caller cannot park the pool on a long decode.

**Playback.** Four rules, stated in the module and kept: decode each
audible span once to disk PCM under a key of path, in-point, duration,
speed, pitch mode and chain (deliberately not volume, fades or position);
one cpal stream mixes the memory-mapped clips with gain applied at mix
time from the one `gain_at`; the device's sample counter is the only clock;
the stream is supervised, rebuilt on failure or a device change with
backoff. Threads: the output supervisor, a 33 ms transport publisher, two
decode workers popping the newest job first and skipping jobs the timeline
no longer wants, and a cache sweep guarded so sweeps never pile up. The
audio callback touches exactly one lock, and only with `try_lock`, because
it must never block. Position is stored *and* messaged on play and seek,
since a paused callback returns before storing.

Every lock in playback and titles recovers from poisoning rather than
propagating it: each guards a cache or a list that is still valid after a
panic elsewhere, and refusing it would end audio or titles for the session.

**Export, templates, jobs, directories.** `Exporter` adapts a progress
closure into the engine's `Reporter` and runs under a `SingleFlight`,
which refuses a second concurrent job and gives each run its own cancel
flag, so a cancel can only stop the job that is running. Templates are
bundles under the config directory (a document with placeholder media, the
assets it needs, a poster); instantiating one refuses missing fills before
creating anything and fills every slot in one `Batch`, so a half-fillable
set of media leaves no half-made project. Deleting a template
canonicalises and checks the path is inside the library. `AppDirs` is the
platform's config and data directories under `app.concat.editor`.

---

## 11. Speech: `concat-speech`

Transcription runs whisper.cpp in-process on a clip's audio decoded to
16 kHz mono, greedy sampling, threads capped at eight, with the job's
cancel flag wired to whisper's abort callback. Six models are known,
nothing above `small`. Text to speech runs Kokoro through sherpa-onnx's C
API, 36 voices, English and Chinese (what the lexicons cover), and writes a
WAV into the project's `audio/` folder because a clip will point at it.
Both download their models on demand into the app's data directory, via a
blocking HTTP client whose read timeout is what makes the cancel flag
reachable, unpacking through a staging folder that refuses paths escaping
it, and renaming into place only when complete. Both are one-at-a-time
through `SingleFlight`. The crate is separate from `concat-host` so its
native libraries stay out of everything that does not speak.

---

## 12. The window: `concat`

**One library, three entry points.** `concat::run()` is the program.
`main.rs` calls it on the desktop and on iOS; `concat-android`'s
`android_main` installs Slint's Android backend and calls it.
`platform.rs` holds the three things a phone does differently: how the
backend is chosen, how a file or folder is picked, and whether there is a
title strip to drag. Everything else is one tree, one state, one set of
callbacks.

**Startup, in order.** The GPU device (`gpu.rs`) is acquired first, on
Metal, DX12 or Vulkan, because Slint takes it at backend selection; with a
device, the renderer and the engine's compositor draw on one device and a
monitor frame is a texture Slint shows as it is. Then `Host::start` brings
up playback, the monitor, the exporter, the transcriber and speech. Then
the `App`, the `Studio`, and the `Shell`: the weak window, the studio in a
`RefCell`, and the models, installed in a thread-local. Every Slint model is
handed over exactly once, since a fresh model is a reset that rebuilds
every row hanging off it. Then about 120 callbacks are bound through three
macros, each "mutate, then republish": the whole window, the lanes alone
(for the handlers a pointer drives as a stream), or the dock alone.

**`studio.rs` is the state.** Its fields group by the line in section 1:
the edit (`session`, the echo, dirty, autosave), the bin (rows with stable
ids, thumbnails, peaks, filmstrips), the view (lane locks and sizes,
selection, playhead, scroll, seconds per pixel, tool, snap, quality,
transport), the sheets (export, settings, menus, toasts), the launch screen
(recents, posters) and the workspace (the dock tree, gestures, stage
guides, drop plans). `project()` returns the echo when there is one, else
the session's project, else an empty one. `apply` sends a command to the
session, turns a refusal into a toast, and on success marks dirty, prunes
the selection, arms a 1.5 s autosave, resyncs audio (flattened clips into
playback's `ClipSpec`s through the same `audio_pieces` the export uses),
requests art and requests a preview.

**The echo.** A press clones the project; the pointer mutates the clone;
`publish` draws whichever exists; release diffs the two and emits exactly
one `MoveClips` or `TrimClip`. The inspector's knobs write the echo and
`clip_commit` diffs it into a batch; a commit with the same key within
900 ms undoes the previous step first, so a dragged knob is one undo. Stage
gestures composite the echo too, snapping against frame lines and other
pictures. Undo therefore undoes the gesture, never a pixel of it.

**`host.rs` is the bridge.** `Shell::with` reaches the state through the
thread-local, which is why it needs no lock: nothing but the event-loop
thread touches it. `spawn(work, then)` runs work on a thread and hops back
through `slint::invoke_from_event_loop`, then publishes, so a completion
never has to remember to redraw. Frames cross threads as `Frame`s, never as
Slint images. The preview is single-flight: one composite at a time, the
latest request wins, the echo is what gets composited when there is one,
and quality scales the frame by 1, ½ or ¼.

**The Slint tree.** `App` is a focus scope (shortcuts, and a click on
nothing blurs a field), the title bar, the start screen or the workspace,
then the export and settings dialogs, tooltips, the app menu and toasts;
declaration order is paint order. `Workspace` reports its own box up and
draws one `Seat` per leaf of the dock tree and one splitter per divider.
A seat is one of four panes (media, preview, inspector, timeline) with a
drop area that previews a split. The timeline pane is tabs, the tool tray,
then headers beside `Lanes`, which draws the bands, the tick grid, one
`Clip` per row (filmstrip tiles, a waveform path, fades, transition
wedges), the drop ghost, the razor guide, the ruler and, last, the
playhead. The preview pane holds the fitted stage with the selection
outlines, corner grips and rotation handle, the frame field, transport
and quality. Positions in the models are seconds, never pixels; drag
deltas are taken in window coordinates because a local delta measures
against its own moving origin; and the ghost is Rust's answer to the
hover, not the panel's guess.

Globals carry what threading would tangle: `Editor` holds every property
and callback the four views use, because two seats showing the same view
must agree; `Theme` is one `dark` switch over two palettes plus the type
roles, the size scale, radii and the content swatches no palette may
decide. `Paint` maps a clip kind to its mark and well, and the bin card
and the timeline clip wear the same two colours, so they read as one
object moved. `dock.rs` is the workspace tree, addressed by path rather
than index, with a default of library beside preview-over-inspector,
above the timeline. `chips.rs` draws drag chips as SVG so Slint rasterises
them with the window's fonts at the window's scale. `prefs.rs` is a small
JSON file of remembered choices. `sysinfo.rs` gathers one list of facts
that feeds both the About rows and the copyable block, so a fact cannot be
on the page and missing from the report. Fonts, the logo and the effect
previews are embedded by `build.rs`.

---

## 13. Build and delivery

- **Toolchain.** Pinned in `engine/rust-toolchain.toml`. `cargo build`
  needs the FFmpeg 7+ development libraries, cmake and a C++ toolchain
  (whisper.cpp), and libclang (bindgen); sherpa-onnx's prebuilt libraries
  are fetched by its crate. Skia needs fontconfig and freetype on Linux.
- **Profiles.** `dev` compiles dependencies at `opt-level = 2` because
  decoding and compositing are unusable unoptimised. `quick` is release
  without LTO, incremental, for trying a change. `app` is the shipping
  binary: fat LTO, `panic = "abort"`, stripped. The workspace `Cargo.toml`
  says what each knob costs and what is deliberately not set.
- **Renderer.** Skia by default; `--no-default-features --features wgpu`
  swaps in FemtoVG over wgpu. The two are meant to be compared. Type
  weights differ by one step between them because FemtoVG composites glyph
  coverage without gamma correction.
- **Workflows.** `ci.yml` runs `fmt`, `clippy -D warnings` (which makes a
  public item without documentation a failure) and the tests, plus the
  web check of the five portable crates. `build-app.yml` builds the window
  on six native runners (macOS arm64 and x86_64, Linux x86_64 and aarch64,
  Windows x86_64 and arm64), bundles, signs and notarises on macOS, and is
  called by `nightly.yml` on every green push and by `release.yml` on
  every `v*` tag. `mobile.yml` cross-builds for Android and iOS.
  `nix.yml` proves the flake, which builds the window with every native
  dependency pinned on Linux.
- **Phones.** FFmpeg is cross-built from source by
  `engine/scripts/ffmpeg-mobile.sh` as static archives with the platform's
  hardware codecs (MediaCodec, VideoToolbox) and nothing else linked;
  sherpa-onnx is fetched as k2-fsa's shared library by
  `scripts/sherpa-mobile.sh`; `scripts/mobile-env.sh` turns what those
  leave under `engine/vendor/` into the build environment and papers over
  three build-script quirks it documents in place. `concat-android` is
  packaged by cargo-apk; `scripts/ios-app.sh` lays out the iOS bundle.
- **Licensing.** The engine is AGPL-3.0-or-later with a plugin exception
  (`LICENSE-EXCEPTIONS.md`); Slint is taken under its GPL-3.0-only option
  (`THIRD_PARTY_NOTICES.md` says why the two combine).

---

## 14. Testing

240 unit tests, all in-file `#[cfg(test)]` modules; there are no `tests/`
directories. The pure layers are where the coverage is dense: rational
arithmetic and the NTSC drift proof, arenas and stale handles, the retime
maths, every command's serde round trip and the tolerant reader's cases,
the expression language, manifest validation and every FFmpeg package's
fixtures (a package without fixtures fails the build), the audio graph's
shape, seek policy and LRU eviction, filter strings, all seven
transition-resolution behaviours, CPU/GPU parity, the speech model
bookkeeping, and the host's cache keys, WAV parsing, sweep ordering and
template packing. Tests that touch FFmpeg encode a small file into the
temporary directory first.

The window has unit tests for its formatting helpers and stage geometry.
The seams that a window drives end to end (publish, the dock tree, the
echo and commit path, playback against a device) are exercised by using
the app, which is the work in 15.1.

---

## 15. Next work

Ordered by how much each matters. Each is either an invariant a
contributor could break without knowing it, or a piece of work with a
known shape.

### 15.1 Put a real project through every flow in the window

Import, place, trim, split, transitions, a title, an effect, a speed
curve, transcription, narration, a template, an export, on each renderer
and each platform. The engine's seams are tested; the window's are used.
What that pass turns up goes above everything below.

### 15.2 A phone layout

The window builds and packages for Android and iOS, and what runs there is
the desktop's tree on a phone's screen. A phone wants one seat at a time,
touch-sized targets and the timeline under the monitor; the `.slint` tree
can carry a second arrangement chosen by the window's size. With it: the
system document picker, since `platform.rs` returns nothing for a pick on
a phone and `concat-media` opens by path; MediaCodec and VideoToolbox
asked for by name (they are linked, and the software decoder is what is
opened); and the monitor on Android, which composites on the CPU because
Slint's Android backend creates its own device.

### 15.3 Two paths that build different chains

On the GPU, effects with a shader leave the FFmpeg chain and travel as
passes; on the CPU they stay in the chain. A machine with an adapter and
one without therefore build different filtergraphs for the same clip, and
the CPU compositor ignores passes by design. Correct today because every
shader-backed package also carries a chain; a WGSL-only package would
render on one machine and not the other. Either require the chain for
`effect` kinds in validation, or teach the CPU path to refuse the package
loudly.

### 15.4 Small things the code already knows about

- `MoveTimeline`'s documented index is counted with the timeline removed;
  the clamp is against the unremoved length.
- `doc.rs` normalises `muted: false` to absent, so an explicit false does
  not round-trip byte for byte.
- `Editor::apply` clones the project before every command, including ones
  that fail or change nothing.
- `Transcriber::download_model` returns early for a present file without
  a final progress report, so a caller waiting on `done` waits.
- FFmpeg-touching tests write fixed file names into the temporary
  directory; two concurrent runs can collide.
- The cross-fade adjacency scan is quadratic and takes the first match.
- `Kind::Transition` and `Kind::Generator` are validated and unused; the
  motion transitions in the inspector are present and disabled.
- The programme level meter has no feed; playback's mix does not report
  levels.
- `ConfigPane` carries a demo-era property group and `ui/demo/` is still
  imported for one type.
- No clip culling beyond the visible flag; a windowing scheme is worth
  writing the day a timeline needs one.

---

## 16. How to change things

- **A new operation on the edit.** Add a `Command` variant in
  `concat-project/src/commands.rs`, make its `apply` report `applied`
  honestly, add it to the serde round-trip test's list, then bind a
  callback in `concat/src/lib.rs` that builds it from the window's state.
- **A new effect.** Add a folder under `concat-effects/packages/` with
  `effect.toml` and `fixtures.toml` (and `effect.wgsl` if it has a
  shader). The build fails until the fixtures cover default, minimum and
  maximum. Nothing in Rust changes.
- **A new pane.** A `PaneKind`, its entry in the `Panes` global, a branch
  in `Seat`, and the `Editor` properties and callbacks it needs.
- **A new decode or encode backend.** Implement `FrameSource` or
  `FrameSink` in `concat-media`. Nothing else in the workspace changes.
- **A new platform.** `platform.rs` for the window's three seams,
  `concat-media/build.rs` for what FFmpeg needs linked, and a job in the
  workflow that builds it.

Reading order for a cold start: this file; then `concat-core/src/lib.rs`,
`concat-project/src/lib.rs` and `concat-export/src/lib.rs`, which between
them hold the vocabulary, the document and the render loop; then
`concat/src/studio.rs` for the window's side of the line; then
`cargo doc --open` for the rest.
