# Repository Guidelines

**Concat** — a free, open-source, cross-platform video editor (CapCut alternative). Rust engine, Slint UI, FFmpeg-linked media I/O, GPU compositor (wgpu). Runs on macOS, Windows, Linux, Android, iOS.

---

## Project Overview

Concat is a native video editor with a Rust engine and Slint-based UI. Core principles:
- **Exact rational time** — all timestamps are `Rational` seconds, never `f64`
- **Generational arenas** — graph handles are `Copy` + generation-checked, no raw pointers
- **FFmpeg in one crate** — `concat-media` is the only crate linking libav*
- **Threaded, not async** — render path is CPU-bound; async buys nothing
- **GPU compositor** — wgpu/Skia, single device shared with Slint window

---

## Architecture & Data Flow

```
concat (UI) → concat-speech → concat-host → {concat-export, concat-project, concat-media} → concat-core
                                                            → concat-render → concat-core
                                                               concat-export → concat-effects → concat-project
                                                   concat-mcp → concat-server → concat-api → concat-host

concat-cli → concat-server → concat-api → concat-host
```

**Crate map** (`src/crates/`):
| Crate | Purpose | Key Deps |
|-------|---------|----------|
| `concat-core` | Time, arenas, frames, timeline model. Vocabulary every crate speaks. | **none** (std only) |
| `concat-media` | Decode/encode via FFmpeg (libavformat, libavcodec, libavfilter, swscale, swresample). Only crate knowing FFmpeg exists. | `concat-core`, `ffmpeg-the-third` |
| `concat-project` | Document model, commands, undo, `concat.json` I/O. | `concat-core`, `serde`, `serde_json` |
| `concat-render` | Timeline + timestamp → frame. Plan (no pixels) → GPU compositor. | `concat-core`, `wgpu` |
| `concat-effects` | Effect packages: WGSL shaders + FFmpeg audio chains. Catalogue + probes. | `concat-project`, `serde`, `toml` |
| `concat-export` | Timeline → file. Frame-by-frame render loop, monitor true frame. | `concat-core`, `concat-media`, `concat-render`, `concat-project`, `concat-effects` |
| `concat-api` | JSON-RPC dispatcher: projects, edits, media, effects, templates, exports, frames. | `concat-host`, `concat-export`, `concat-effects`, `concat-project` |
| `concat-server` | API on socket: JSON-RPC (TCP/Unix) + gRPC (feature `grpc`). | `concat-api`, `tonic`, `tokio` |
| `concat-cli` | `probe`, `render`, `api`, `serve` (socket), `preview`, `check`. `--features grpc` for `serve --grpc`. | `concat-server`, `concat-api`, engine crates |
| `concat-host` | Sessions, project folders, previews, playback, templates, job slots, model downloads. | `concat-media`, `concat-project`, `concat-export`, `cpal` |
| `concat-speech` | Transcription (whisper.cpp) + TTS (Chatterbox Turbo or the system voice, over ONNX Runtime). | `concat-host`, `concat-media`, `whisper-rs`, `ort` |
| `concat` | Editor window (Slint). Every pane, dialog, primitive. `concat --mcp` serves MCP to AI agents. | `slint`, `concat-host`, `concat-speech`, `concat-mcp`, `concat-server` |
| `concat-mcp` | MCP server over stdio. Tools are concat-api methods, schemas generated from request types. | `concat-api`, `concat-server`, `serde_json` |
| `concat-android` | Android activity hosting the window. Also ships `concat-mcp-android` and `concat-api-server-android` binaries. | `concat`, `slint`, `concat-mcp` |
| `concat-mobile` | Spike: UniFFI cdylib behind a Kotlin + Compose phone shell. | `concat-host`, `concat-project`, `concat-export`, `concat-media`, `wgpu` |
| `concat-text` | Fonts, presets, text layout, titles as pictures. | `concat-core` |
| `concat-vision` | Cutout (ONNX), masks, inference. `infer` feature gates ONNX Runtime. | `concat-project`, `ort` |

---

## Key Directories

```
src/
├── crates/
│   ├── concat-core/          # Time, arena, frame, timeline, mask, retime, shader
│   ├── concat-media/         # Decode, encode, audio, hardware, pool, prefetch, probe, peaks, feed, SVG
│   ├── concat-project/       # Project, timeline, clip, track, commands, undo, IO
│   ├── concat-render/        # Plan, GPU compositor, upscale, transitions, metrics
│   ├── concat-effects/       # Packages (WGSL + FFmpeg), catalogue, probes, cards
│   ├── concat-export/        # Flatten, render loop, export job, monitor frame
│   ├── concat-api/           # Methods: project, edit, media, catalogue, template, export, frame
│   ├── concat-server/        # JSON-RPC lines, gRPC, auth token, dispatcher
│   ├── concat-cli/           # probe, render, api, serve commands
│   ├── concat-host/          # Session, preview, playback, templates, jobs, models, logs, memory
│   │   └── tests/export.rs   # End-to-end export scenario (every_edit_still_exports)
│   ├── concat-speech/        # Whisper (in-process), Chatterbox TTS, download/verify models
│   ├── concat-text/          # Fonts, presets, text layout, titles as pictures
│   ├── concat-vision/        # Cutout (ONNX), masks, inference
│   ├── concat/               # Slint UI: panes, studio, lib.rs (huge), main.rs, platform, fonts, i18n
│   │   ├── build.rs          # Compiles ui/app.slint, embeds fonts+images+cards, generates text presets
│   │   ├── ui/app.slint      # Main Slint UI tree
│   │   ├── src/panes/        # Individual UI panes (timeline, media, settings, etc.)
│   │   ├── src/studio/       # Studio state (timeline model, project view)
│   │   ├── src/gpu.rs        # Shared wgpu device creation, handed to Slint + compositor
│   │   ├── src/platform.rs   # Desktop/iOS/Android backend differences
│   │   ├── src/i18n.rs       # Locale system (en.json → locales)
│   │   ├── src/launch.rs     # CLI arg parsing: --mcp, --smoke, --whisper-cpu, --software, --version, --project
│   │   ├── text-presets/     # Built-in title presets (concat.bold-caption, etc.)
│   │   ├── cards/            # Effect card images
│   │   └── locales/          # en.json and translated locale JSON files
│   ├── concat-android/       # Android activity, JNI glue
│   ├── concat-mobile/        # UniFFI spike for Kotlin + Compose shell
│   ├── concat-mcp/           # MCP server over stdio
│   ├── concat-cli/           # probe, render, api, serve commands
│   └── ...
├── .cargo/config.toml        # Cargo aliases, profile config
├── .config/nextest.toml      # cargo-nextest CI profile
├── rust-toolchain.toml       # Pinned to 1.93.1
├── Cargo.toml                # Workspace root
├── Cargo.lock
├── rustfmt.toml
└── scripts/                  # locales.py, models.py, ffmpeg-mobile.sh, mobile-env.sh, ios-app.sh
```

---

## Development Commands

**Work from `src/`:**
```sh
# Run editor (debug)
cargo run -p concat

# Faster rebuilds for trying changes (profile quick: release opts, concat crate at opt-level 0)
cargo run --profile quick -p concat

# Shorthand alias (see src/.cargo/config.toml): same as above
cargo dev

# Shipping binary (fat LTO, panic=abort, stripped)
cargo build --profile app -p concat

# Format + lint (what CI checks)
cargo fmt --check && cargo clippy --all-targets -- -D warnings

# Tests (cargo-nextest in CI; cargo test locally works)
cargo test                          # all
cargo test -p concat-core           # single crate
cargo nextest run --workspace       # with nextest (CI profile: --profile ci)

# CLI driver
cargo run -p concat-cli -- probe video.mp4
cargo run -p concat-cli -- render in.mp4 out.mp4 --frames 120
cargo run -p concat-cli -- api '{"method":"catalogue.list","kind":"filter"}'
cargo run -p concat-cli -- preview   # draw one effect's card
cargo run -p concat-cli -- check     # load effect packages, run probes on GPU
cargo run -p concat-cli -- serve     # JSON-RPC on 127.0.0.1:7420
cargo run -p concat-cli --features grpc -- serve --grpc 127.0.0.1:7421

# The `concat` binary's own launch flags
cargo run -p concat -- --version
cargo run -p concat -- --smoke      # CI smoke: start up and exit, no window
cargo run -p concat -- --whisper-cpu  # load whisper CPU kernels, print backend, exit
cargo run -p concat -- --software   # software-rendered window (safe graphics mode)
cargo run -p concat -- --mcp        # serve MCP to AI agents (docs/MCP.md)
cargo run -p concat -- --mcp window|headless|auto
cargo run -p concat -- --project /path/to/project

# Locale sync (after changing UI strings)
python3 ../scripts/locales.py

# Model manifest check
python3 ../scripts/models.py --check
```

**Requirements:**
- Rust 1.93.1 (pinned in `rust-toolchain.toml`)
- FFmpeg 7+ dev libraries (`brew install ffmpeg` on macOS; BtbN shared build + `FFMPEG_DIR` on Windows/Linux)
- cmake + C++ compiler (whisper.cpp)
- Linux: `libfontconfig1-dev libfreetype6-dev libxkbcommon-dev libgl1-mesa-dev libgtk-3-dev libasound2-dev libclang-dev libvulkan1 mesa-vulkan-drivers`

---

## Code Conventions & Common Patterns

### Time Handling
```rust
use concat_core::time::{Rational, FrameRate, TimeRange};

let rate = FrameRate::NTSC_30;          // 30000/1001
let t = rate.time_of_frame(1800);       // Rational { num: 60060, den: 1000 }
let frame = rate.frame_at(t);           // round-trips exactly
```
- **Never use `f64` for timestamps**. Only convert at display/FFmpeg boundaries.
- `Rational` uses `i128` arithmetic, reduced to `i64` — panics on overflow.

### Generational Arenas
```rust
use concat_core::arena::{Arena, Id};

let mut arena = Arena::new();
let id: Id<Clip> = arena.insert(Clip::new());
arena.get(id);           // Option<&Clip>
arena.remove(id);        // stale Id returns None, not UB
```
- `Id<T>` is `Copy`, `Eq`, `Hash` — safe for undo records, cross-thread.
- Phantom `T` prevents mixing arenas (clip Id into track arena fails at compile time).

### Error Handling
- `thiserror` for crate-level error enums (`concat_core`, `concat_media`, etc.)
- Public API errors carry `data.code` (e.g., `"notOpen"`, `"refused"`, `"busy"`) for programmatic handling.
- Log via `log` facade (`log::warn!`, `log::error!`) — writes to `<app data>/logs/concat-<when>.log`. `CONCAT_LOG=debug` raises level.

### Threading Model
- **No async in engine**. Render, decode, export are CPU-bound; threads via `std::thread` or `rayon`.
- `concat-host` runs preview/playback on dedicated threads; `cpal` audio callback on its own.
- `concat-server` JSON-RPC uses blocking I/O per connection; gRPC uses `tokio` (feature-gated).
- `concat-mcp` serves MCP over stdio, blocking.

### Dependency Injection
- Crates expose traits in `lib.rs` where callers need swappable implementations (e.g., `Compositor` in `concat-render`).
- Features gate heavy deps: `gpu` on `concat-render`/`concat-export`/`concat-host`/`concat`; `grpc` on `concat-server`/`concat-cli`; `schema` on `concat-project`/`concat-api`; `infer` on `concat-vision`; `local-socket` on `concat-server`/`concat-mcp`/`concat-android`; `chatterbox` on `concat-speech` (used by `concat`).

### State Management
- Project model (`concat-project`) is `Arc`-backed; commands produce new snapshots for undo.
- UI (`concat`) holds `Arc<Project>` and dispatches commands via `concat-project` command types.
- Host (`concat-host`) owns `Session` → `ProjectHandle` → caches (previews, waveforms, artwork).

### File Headers
```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah
```
Every source file must have this exact header. No personal copyright lines.

---

## Important Files

| Path | Purpose |
|------|---------|
| `src/Cargo.toml` | Workspace root: versions, profiles (quick, app), patches (slint-skia, whisper-rs-sys) |
| `src/rust-toolchain.toml` | Pinned toolchain 1.93.1 |
| `src/.config/nextest.toml` | Test profile (per-test timeout, JUnit) |
| `src/.cargo/config.toml` | `cargo dev` alias; warns about FFMPEG_DIR and incremental caches |
| `src/crates/concat-core/src/lib.rs` | Core re-exports: `Rational`, `FrameRate`, `TimeRange`, `Arena`, `Id`, `Frame`, `Timeline`, `Clip`, `Track`, `Blend`, `ShaderPass`, `Lut` |
| `src/crates/concat-media/src/decode.rs` | Video/audio decode, hardware accel, frame pool |
| `src/crates/concat-render/src/plan.rs` | FramePlan: what's on screen, from where, how strongly (no pixels) |
| `src/crates/concat-render/src/gpu.rs` | WgpuCompositor: draws every frame on shared wgpu device |
| `src/crates/concat-render/src/reference.rs` | CPU compositor oracle (GPU parity suite) |
| `src/crates/concat-render/src/transitions.rs` | Transition parity tests |
| `src/crates/concat-export/src/lib.rs` | Export job, render loop, monitor true frame |
| `src/crates/concat-project/src/lib.rs` | Project, Timeline, Clip, Track, commands, undo, JSON I/O |
| `src/crates/concat-project/src/commands.rs` | Command apply/undo tests |
| `src/crates/concat-api/src/lib.rs` | All API methods, request/response types, JSON-RPC dispatch |
| `src/crates/concat-server/src/lib.rs` | Socket server, JSON-RPC dispatcher, gRPC transport |
| `src/crates/concat-mcp/src/lib.rs` | MCP server: tools are API methods, schemas from `schemars` |
| `src/crates/concat/src/lib.rs` | Main UI crate: `run()`, `run_with_project()`, `smoke()`, window wiring |
| `src/crates/concat/src/main.rs` | Binary entry: `launch::parse`, `--mcp`, `--smoke`, `--whisper-cpu`, etc. |
| `src/crates/concat/src/launch.rs` | CLI arg parsing for the `concat` binary |
| `src/crates/concat/src/gpu.rs` | Shared wgpu device creation, handed to Slint + compositor |
| `src/crates/concat/src/platform.rs` | Desktop/iOS/Android backend differences |
| `src/crates/concat/src/i18n.rs` | Locale system (en.json → locales) |
| `src/crates/concat/build.rs` | Compiles ui/app.slint, embeds fonts/images/cards, generates text presets |
| `src/crates/concat/ui/app.slint` | Main Slint UI tree |
| `src/crates/concat/text-presets/` | Built-in title presets |
| `src/crates/concat/cards/` | Effect card images |
| `src/crates/concat/locales/` | en.json and translated locale JSON files |
| `src/crates/concat-host/tests/export.rs` | End-to-end export scenario (`every_edit_still_exports` — long, special timeout on Windows) |
| `src/crates/concat-host/tests/stabilize.rs` | Stabilize test |
| `.github/workflows/ci.yml` | CI: fmt, clippy, nextest, GPU parity, gRPC feature |
| `.github/workflows/build-app.yml` | Release builds: macOS, Windows, Linux (app profile) |
| `.github/workflows/mobile.yml` | Android (APK) + iOS builds |
| `scripts/locales.py` | Syncs `en.json` → all locale files |
| `scripts/models.py` | Validates `models/manifest.toml` ↔ engine tables |

---

## Runtime/Tooling Preferences

- **Rust**: 1.93.1 (pinned). Edition 2024.
- **Package manager**: Cargo. Workspace under `src/`.
- **Build profiles**:
  - `dev`: opt-level 2 for engine crates (decoding unusable at 0)
  - `quick`: release opts + incremental, concat crate at opt-level 0 (fast UI rebuilds)
  - `app`: fat LTO, panic=abort, strip=symbols — shipping binary
  - `release`: standard release (for profiling)
  - `ci`: test profile (opt-level 2, no debug info)
- **Formatter**: `rustfmt` (workspace config in `rustfmt.toml`, max_width=100)
- **Linter**: `clippy` with `-D warnings` (missing_docs = warn; too_many_arguments = allow)
- **Test runner**: `cargo-nextest` (CI), `cargo test` (local OK)
- **UI framework**: Slint 1.18 (Skia renderer, wgpu backend, unstable-wgpu-30, unstable-fontique-011)
- **GPU**: wgpu 30, pollster. Single device shared with Slint.
- **FFmpeg**: `ffmpeg-the-third` 6, linked (not spawned). Features: codec, filter, format, software-resampling, software-scaling.
- **ONNX Runtime**: ort 2.0.0-rc.13, dynamic link, CoreML/DirectML/NNAPI backends.
- **Mobile**: NDK (Android), Xcode (iOS). Cross-compile FFmpeg via scripts; ONNX Runtime is fetched prebuilt for the target by the `ort` crate.

---

## Testing & QA

**Run tests:**
```sh
cargo test                          # all crates
cargo test -p concat-render         # single crate
cargo nextest run --workspace --profile ci   # CI profile (per-test timeout, JUnit)
```

**Test organization:**
- Unit tests inline (`#[cfg(test)] mod tests { ... }`) — most crates.
- Integration-style: `concat-render` has `reference::CpuCompositor` (CPU oracle) + GPU parity suite.
- `concat-host/tests/export.rs` has scenario tests exporting real timelines (one scripted editing session, ~30 exports).
- `concat-project` tests command apply/undo/IO round-trips.
- `concat-api` tests JSON-RPC dispatch + gRPC transport.
- Doc examples: `cargo test --workspace --doc -j 1`

**CI checks (must pass locally before PR):**
```sh
cd src && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
python3 ../scripts/locales.py --check
python3 ../scripts/models.py --check
```

**Coverage expectations:**
- No enforced coverage threshold.
- Engine crates: test invariants, boundaries, transitions, precedence, errors.
- UI: no unit tests (Slint compiler output); rely on integration scenarios in `concat-host/tests/export.rs` + manual QA.
- GPU parity suite runs in CI (lavapipe on Linux, WARP on Windows, skipped on macOS CI).
- `every_edit_still_exports` has its own step on Windows (longer timeout, run alone) because it has hung there.

**Important env vars for tests:**
- `CONCAT_REQUIRE_GPU=1` — GPU tests fail rather than skip when no adapter is usable
- `FFMPEG_DIR` — path to unpacked FFmpeg dev build
- `ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK=1`, `LD_LIBRARY_PATH` — ONNX Runtime shared lib
- `CONCAT_LOG=debug` — raise log level
- `CARGO_PROFILE_DEV_DEBUG=0` — no debug info in test binaries (smaller, faster link)

---

## Quick Reference

```sh
# Most common dev loop
cd src
cargo dev                          # edit, rebuild ~20s
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo nextest run --profile ci     # full test suite

# Add a locale key
python3 ../scripts/locales.py      # updates all locale files from en.json

# Add a downloadable model
# 1. Add to models/manifest.toml
# 2. Add to engine table (concat-speech, concat-vision)
# 3. python3 ../scripts/models.py --check
# 4. Maintainer runs "Mirror models" workflow
```