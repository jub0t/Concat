# Concat Engine

The video engine behind Concat. Rust, no GC, no hidden control flow.

## Layout

| Crate | What it is | Dependencies |
|---|---|---|
| `concat-core` | Time, arena, frames, timeline model. The vocabulary every other crate speaks. | **none** (std only) |
| `concat-media` | Getting pixels and samples in and out of files. Links FFmpeg (libav*), and is the only crate that knows it exists. | `concat-core`, ffmpeg-the-third |
| `concat-project` | The edit itself: document model, operations as commands, undo, concat.json IO. | serde, serde_json |
| `concat-render` | Turning a timeline plus a timestamp into one finished frame. | `concat-core` |
| `concat-cli` | A binary to drive the above. The vertical slice. | all of them |
| `concat-host` | What the window needs that is not the edit: sessions, project folders, previews, playback, templates, job slots. | `concat-media`, `concat-project`, `concat-export`, cpal |
| `concat-speech` | Transcription (whisper.cpp, in-process) and text to speech (Kokoro via sherpa-onnx). | `concat-host`, `concat-media`, whisper-rs, sherpa-onnx |
| `concat` | The editor window: every pane, dialog and primitive, in Slint. The app a user launches. | slint (engine wiring is next) |

The dependency arrows point one way: `{concat, cli} -> {media, render} -> core`.
If you ever find yourself wanting `core` to depend on `media`, something has
been put in the wrong crate.

`concat` replaces the Tauri + React app in `../desktop`, which is deprecated
and stays only until this window can do everything it did. The window was
built in the wc-ui-rnd repository against the same design as the React
editor; what landed here is the whole UI driven by demo data, and the work
now is to hand it the engine's real project, preview and export.

## Build and run

```sh
cargo build
cargo test
cargo run -p concat-cli -- probe some-video.mp4
cargo run -p concat-cli -- render some-video.mp4 out.mp4 --frames 120
cargo run -p concat                    # the editor window, debug
cargo build --profile app -p concat    # the shipping binary: fat LTO, panic=abort, stripped
```

Nothing is spawned at run time: FFmpeg is linked, whisper.cpp and sherpa-onnx
are compiled in. A build needs the FFmpeg 7+ development libraries (headers
and import libraries) - `brew install ffmpeg` on macOS, a BtbN `shared` build
unpacked and pointed at with `FFMPEG_DIR` on Windows and on Linux
distributions whose packaged FFmpeg is older than 7 - plus cmake and a C++
toolchain for whisper.cpp. The window builds with Slint's
Skia renderer by default; `--no-default-features --features wgpu` swaps in
FemtoVG over wgpu, and the two are meant to be compared, not chosen once.
On Linux, Skia needs the fontconfig and freetype headers at build time (see
`.github/workflows/build-app.yml` for the package list).

## Reading this codebase cold

1. `docs/decisions/` - short notes on why things are the way they are. Read these first.
2. `crates/*/src/lib.rs` - every crate opens with a `//!` block saying what it is for.
3. `cargo doc --open` - the generated API map of the whole engine.

## Conventions

- **Time is exact.** All timestamps are `concat_core::time::Rational` seconds, never `f64`.
  Frame-accurate editing and floating point do not mix.
- **Graphs use handles, not pointers.** See decision 0003.
- **Shallow generics.** Concrete types until three call sites demand otherwise.
- **Threads, not async.** The render path is CPU-bound; `async` buys nothing here.
