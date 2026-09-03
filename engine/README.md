# Concat Engine

The video engine behind Concat. Rust, no GC, no hidden control flow.

## Layout

| Crate | What it is | Dependencies |
|---|---|---|
| `concat-core` | Time, arena, frames, timeline model. The vocabulary every other crate speaks. | **none** (std only) |
| `concat-media` | Getting pixels and samples in and out of files. Links FFmpeg (libav*), and is the only crate that knows it exists. | `concat-core`, ffmpeg-the-third |
| `concat-project` | The edit itself: document model, operations as commands, undo, concat.json IO. | serde, serde_json |
| `concat-render` | Turning a timeline plus a timestamp into one finished frame. | `concat-core`, wgpu (optional) |
| `concat-export` | Timeline to file: flatten, the filter-chain builders, the frame-by-frame render loop, the paused monitor's true frame. | `concat-core`, `concat-media`, `concat-render`, `concat-project` |
| `concat-cli` | A binary to drive the above. The vertical slice. | the engine crates |
| `concat-host` | What the window needs that is not the edit: sessions, project folders, previews, playback, templates, job slots. | `concat-media`, `concat-project`, `concat-export`, cpal |
| `concat-speech` | Transcription (whisper.cpp, in-process) and text to speech (Kokoro via sherpa-onnx). | `concat-host`, `concat-media`, whisper-rs, sherpa-onnx |
| `concat` | The editor window: every pane, dialog and primitive, in Slint. The app a user launches. | slint, `concat-host`, `concat-speech` |

The dependency arrows point one way:

```
concat -> concat-speech -> concat-host -> {export, project, media} -> core
                                       -> render -> core
```

If you ever find yourself wanting `core` to depend on `media`, something has
been put in the wrong crate.

`concat` is the app. It opens project folders through `concat-host`, reads
the engine's project to draw the bin and the lanes, and writes every edit as
a `concat-project` command.

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

On Nix, the flake at the repository root builds the window with every native
dependency pinned: `nix build` (then `./result/bin/concat`), `nix run`, or
`nix develop` for a shell with the toolchain and libraries in it. Linux
x86_64 and aarch64.

## Reading this codebase cold

1. [`../ARCHITECTURE.md`](../ARCHITECTURE.md) - the map of the whole system. Read it first.
2. `crates/*/src/lib.rs` - every crate opens with a `//!` block saying what it is for.
3. `cargo doc --open` - the generated API map of the whole engine.

## Conventions

- **Time is exact.** All timestamps are `concat_core::time::Rational` seconds, never `f64`.
  Frame-accurate editing and floating point do not mix.
- **Graphs use handles, not pointers.** `concat_core::arena` explains why.
- **Shallow generics.** Concrete types until three call sites demand otherwise.
- **Threads, not async.** The render path is CPU-bound; `async` buys nothing here.
