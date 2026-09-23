# Concat — Engineering Handoff

**Date**: 2026-09-19  
**Status**: Builds clean · Tests passing · All changes uncommitted on `main`

---

## 1. What Is Concat

Concat is a **native, offline-first video editor** built with **Rust + Slint UI**, aiming for the feature set of the mainstream consumer editors. It runs on macOS, Windows, Linux, iOS, and Android from a single codebase. There is no cloud dependency, no sign-in, and no telemetry — everything runs on-device.

The project is in active early development. The goal is a free, open editor with the same snappiness and creative breadth as the commercial ones.

---

## 2. Repository Layout

```
Concat/                          ← git repo root
├── src/                         ← Cargo workspace root (cd here for ALL cargo commands)
│   └── crates/
│       ├── concat/              ← Main desktop app (Slint UI + Rust glue)
│       │   ├── src/
│       │   │   ├── lib.rs       ← All UI→Rust callback wiring (on_* handlers)
│       │   │   └── studio.rs    ← Core editor state machine (~8 000 lines)
│       │   └── ui/
│       │       ├── workspace/
│       │       │   └── media-pane.slint   ← Library panel (Effects/Transitions/Filters/etc.)
│       │       ├── timeline/
│       │       │   └── lanes.slint        ← Timeline lanes, clip cards, transition seams
│       │       └── inspector/
│       │           └── clip-inspector.slint ← Right-panel inspector
│       ├── concat-effects/      ← Package system: all effects/filters/transitions/audio
│       │   ├── packages/        ← 165 packages, one folder each (auto-discovered at build)
│       │   └── src/
│       │       ├── catalogue.rs ← Category pill filtering logic
│       │       ├── manifest.rs  ← effect.toml schema + Kind enum
│       │       └── expr.rs      ← Tiny template language for FFmpeg chain strings
│       ├── concat-export/       ← FFmpeg-based export pipeline
│       │   └── src/lib.rs       ← resolve_transitions(), composite_treated()
│       ├── concat-project/      ← Project model, commands, animation presets
│       │   └── src/animation.rs ← All animation keyframe presets
│       ├── concat-render/       ← GPU compositor (wgpu), transition shader runtime
│       │   └── src/gpu.rs       ← GPU regression tests (run with --features gpu)
│       ├── concat-core/         ← TransitionShader / TransitionPass types
│       ├── concat-api/          ← Stable public API
│       ├── concat-cli/          ← CLI front-end
│       ├── concat-media/        ← Media probe & decode
│       ├── concat-server/       ← HTTP server / MCP integration
│       ├── concat-speech/       ← TTS / captions / speech
│       ├── concat-text/         ← Text rendering
│       └── concat-vision/       ← Background removal, segmentation models
├── engine/                      ← Native rendering engine (non-Cargo)
├── assets/                      ← App icons, splash screens
├── models/                      ← Bundled AI models (segmentation, etc.)
└── scripts/                     ← Tooling scripts (locale checker, etc.)
```

> **CRITICAL**: The Cargo workspace root is `src/`, not the git repo root. Every `cargo` command must be run from `src/`.

---

## 3. Dev Setup

```bash
# Prerequisites: Rust stable, FFmpeg (system), wgpu-compatible GPU driver

git clone <repo>
cd "Concat/src"

# Run the desktop app
cargo run -p concat

# Faster iteration build profile
cargo run --profile quick -p concat

# Full test suite (includes GPU shader tests)
cargo test -p concat-effects -p concat-export -p concat-render -p concat-project --features gpu

# Type-check the whole app (including Slint UI macros) — ~45s
cargo check -p concat

# Lint everything
cargo clippy -p concat -p concat-export -p concat-effects -p concat-render -p concat-project \
  --features concat-render/gpu -- -D warnings
```

> **Note for sandbox/CI environments**: `cargo check` and `cargo test` require network access to `index.crates.io`. They will fail in a fully sandboxed environment even with `--offline`. Always run these commands with real network access.

---

## 4. Architecture — Things You Must Know

### 4.1 The Package System (Effects, Filters, Transitions, Audio)

Every built-in effect/filter/transition/audio item is a **folder** in `src/crates/concat-effects/packages/`. Naming convention: `concat.<name>/`.

Each folder contains:
- `effect.toml` — manifest (id, name, kind, params, category tags)
- `main.wgsl` or `effect.wgsl` — GPU shader (required for transitions; optional for effects/filters which have a CPU fallback)
- `ffmpeg.txt` or `ffmpeg.wgsl` — CPU-path FFmpeg filtergraph string (effects and filters; **forbidden** for transitions)
- `fixtures.toml` — golden test fixtures pinning exact FFmpeg chain output

**`build.rs` auto-discovers every folder at compile time.** Adding a package = adding a folder. Nothing in Rust ever names individual packages by ID. `Catalogue::builtin()` is the runtime entry point.

### 4.2 The `Kind` Enum (`manifest.rs`)

| Kind | GPU | FFmpeg | Intensity Slider | Notes |
|---|---|---|---|---|
| `Effect` | ✅ | ✅ | ❌ (define your own params) | Applied to selected clip's chain |
| `Filter` | ✅ | ✅ | ✅ (free, auto, don't declare it) | Overlay layer on timeline |
| `Transition` | ✅ ONLY | ❌ forbidden | ❌ | Two-input shader, `transition()` fn |
| `Audio` | — | ✅ | depends | Applied to clip's audio chain |
| `Generator` | reserved | — | — | No packages exist yet |

### 4.3 Transition Runtime

The GPU transition pipeline spans four crates:

```
effect.toml [transition] → TransitionShader (concat-core)
    → TransitionPass (concat-render GPU compositor)
        → Compositor::combine() called per-frame during preview
            → resolve_transitions() + composite_treated() (concat-export)
                → exported via FFmpeg frame-by-frame
```

Without a GPU the same `Compositor::combine()` runs on `CpuCompositor`, which
draws the shape the manifest names as `xfade` in plain arithmetic
(`concat-render/src/transitions.rs`): the crossfades, the clock wipe, the iris,
the wipes and slides, the zoom and the mosaic, each in the shipped shader's own
terms rather than FFmpeg's reading of the name. The `TransitionPass` carries
the name (`xfade`), filled in by `Catalogue::transition_pass()`. A package
naming no shape, or one the CPU does not draw, gets the dissolve ramp the
incoming clip already carries. The monitor takes the same path: with a GPU a
live transition goes through `PreviewSources::composite()` on the device
(`Monitor::texture_of`, the `needs_cpu` branch), without one through the CPU
shapes.

A transition shader body declares:
```wgsl
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    // from_at(uv) = outgoing clip pixel
    // to_at(uv)   = incoming clip pixel
    // progress    = 0.0 (start of transition) → 1.0 (end)
}
```

**Handle constraint** (not a bug): a dissolve needs real unused source footage ("handle") before/after a clip's trim point. `Studio::apply_transition()` computes available handle and clamps or refuses. `resolve_transitions()` previously had a double-clamp bug that made it impossible to add transitions to untrimmed clips — this was fixed (see §6.3).

### 4.4 UI Callback Wiring

Every UI event flows:
```
Slint UI callback → lib.rs on_* handler → studio.rs method → apply(Command) → state mutation → notify()
```

- `src/crates/concat/src/lib.rs` — **All** UI→Rust wiring lives here. ~1 400 lines of `editor.on_*` registrations.
- `src/crates/concat/src/studio.rs` — The editor state machine. Nearly everything goes through `self.apply(Command::...)`.

### 4.5 Effects vs Filters vs Audition (IMPORTANT)

This distinction caused a critical bug and is non-obvious:

| | Applied where | Single-click behaviour | Double-click / `+` |
|---|---|---|---|
| **Effect** (tab 3) | Selected clip's `video_effects` chain | ✅ Applies directly | N/A (same) |
| **Filter** (tab 4) | Overlay layer placed at playhead | Auditions on monitor (overlay preview only) | Places as layer |
| **Transition** (tab 2) | Cut between two adjacent clips | ✅ Applies directly | N/A (same) |
| **Audio** (tab 3, audio mode) | Selected clip's `filters` chain | ✅ Applies directly | N/A (same) |

**The `audition` system** (`self.audition: Option<String>` in `studio.rs`, `audition_catalogue()`) renders an effect/filter overlay on the monitor **without touching the project**. It was designed for **Filters only**. If you wire `auditions: true` + `auditioned(id) => { root.audition-filter(id); }` to the Effects or Transitions shelf, single-click will flash the effect across the entire monitor frame (looks like "applied to the imported file"). **Do not do this.** The Filters shelf correctly uses `auditions: true`; Effects and Transitions shelves must use `auditions: false`.

### 4.6 Category Pill Bar

The Effects shelf has a horizontal pill category bar:

`All | Featured | Retro & Film | Optical & Lens | Distortion & Glitch | Party & Club | Light & Shadow`

- `matches_category(id, cat)` in `catalogue.rs` maps package IDs to categories
- `Library::category_changed()` (Slint global) + `Studio::library_category()` drive the filter
- When `category == "All"`, `view.group` is set to `-1` to bypass the group filter

### 4.7 The `{...}` Template Language in FFmpeg Chains

`expr.rs` implements a small expression language for FFmpeg filtergraph strings. Supported: `if / clamp / min / max / abs / floor / ceil / round / sqrt / pow / fixed`, arithmetic, comparisons. **No trig (`sin/cos/atan2`).** For per-pixel math (ripple, warps), use FFmpeg's own `geq` filter's expression language instead — it has `sin/hypot/T/X/Y/W/H/r/g/b`.

> ⚠️ Always spot-check a `geq` chain against a real `ffmpeg` binary before trusting it:
> ```bash
> ffmpeg -f lavfi -i testsrc=size=64x64:rate=1:duration=1 -vf "<chain>" -frames:v 1 -f null -
> ```

---

## 5. The Effects Library (165 Packages)

### 5.1 Transitions (19 packages)

| Package ID | Name |
|---|---|
| `concat.blur-dissolve` | Blur Dissolve |
| `concat.film-burn` | Film Burn |
| `concat.zoom-transition` | Zoom |
| `concat.slide` | Slide (L/R/U/D enum) |
| `concat.push` | Push (L/R/U/D enum) |
| `concat.spin` | Spin |
| `concat.swirl` | Swirl |
| `concat.glitch-transition` | Glitch |
| `concat.rgb-split-wipe` | RGB Split Wipe |
| `concat.linear-wipe` | Linear Wipe (L/R/U/D enum) |
| `concat.iris` | Iris |
| `concat.shape-wipe` | Shape Wipe (Circle/Heart/Star enum) |
| `concat.light-leak-transition` | Light Leak |
| `concat.cube` | Cube |
| `concat.clock-wipe` | Clock Wipe |
| `concat.whip-pan` | Whip Pan |
| `concat.zoom-blur-transition` | Zoom Blur |
| `concat.light-leak-flash` | Light Leak Flash |
| `concat.glitch-warp` | Glitch Warp |

### 5.2 Video Effects — Modern Pack (19 packages added in recent session)

**Featured**: `concat.camera-shake`, `concat.rgb-glitch`, `concat.sparkle`, `concat.light-leak`, `concat.tilt-shift`, `concat.strobe-flash`

**Cinematic & Film**: `concat.crt-scanlines`, `concat.film-reel`, `concat.camcorder-90s`, `concat.halation`

**Distortion & Optical**: `concat.wave-warp`, `concat.vortex-swirl`, `concat.prism-dispersion`, `concat.fisheye`, `concat.mirror-tile`

**Party & Club**: `concat.bass-shockwave`, `concat.laser-beams`, `concat.neon-glow`, `concat.color-cycle`

### 5.3 Fixtures Convention

Every package with an `[ffmpeg]` chain must have a `fixtures.toml` with:
- `[[case]] at = "default"` — always required
- `[[case]] at = "min"` and `[[case]] at = "max"` — required if the package has any `[[param]]`

Enforced by the test `every_ffmpeg_package_pins_its_default_and_every_slider_bound` in `concat-effects/src/lib.rs`.

**Fastest way to generate fixtures**: write a temporary `#[test]` that loads via `Catalogue::builtin()` and prints `package.ffmpeg_fragment(&params, index)` with `--nocapture`, paste into `fixtures.toml`, then delete the test.

---

## 6. Recent Work (This Session — All Uncommitted)

### 6.1 Transition Engine Fix (`concat-export/src/lib.rs`)
- Removed a double-clamp bug in `resolve_transitions()` that prevented transitions on untrimmed clips
- Source start is now clamped to `.max(0.0)` only (was also clamping against `b.source_start / b.speed`)
- Added tests: `a_cross_fade_on_untrimmed_clip_succeeds`, `a_packaged_transition_on_untrimmed_clip_succeeds`

### 6.2 Transition Seam UI Redesign (`lanes.slint`)
- Symmetric cut placement (seam spans `-half-w` to `+half-w`)
- 18×18px rounded square seam button `[ ⧗ ]` centered at the cut
- Left/right draggable slider handles (3px wide, 14px hit target, `ew-resize` cursor)
- Floating duration tooltip pill
- Fixed `if` conditional blocks to `visible` properties for proper TouchArea access

### 6.3 Live Animated Previews (`clip-inspector.slint`)
- Added `id` field to `AnimCardData`
- Live procedural hover animations per type (Zoom In/Out, Slide Up/Down/Left/Right, Spin, Pulse, Fade, Bounce, Shake, Wobble, Sway, etc.)

### 6.4 Card Hover Animations + Category Pills (`media-pane.slint`)
- `sweep` animation (wipe overlay) on transition cards on hover
- `shimmer` gradient overlay on effect cards on hover
- `CategoryPillBar` component added for the effects shelf
- Preview artwork mapped for all new packages via `art-of(id)`

### 6.5 Effects/Transitions Click Bug Fix (`media-pane.slint`)
- **Bug**: `auditions: true` was set on the Effects and Transitions shelves, causing single-click to invoke `audition_catalogue()` — which renders an effect overlay across the entire monitor frame. This looked like "the effect applied to the imported file itself."
- **Fix**: Set `auditions: false` on both the Effects shelf (tab 3) and Transitions shelf (tab 2). Single-click now applies directly to the selected clip/cut. Only the Filters shelf (tab 4) retains `auditions: true` (correct for color grade preview).

---

## 7. Known Open Issues / Next Steps

### 🔴 Critical / High Priority

None open. The three that were here are resolved:

| # | Was | Now |
|---|---|---|
| 1 | Transition seam drag handles did not write back to the project | Wired: a press on a handle or the seam button is `clip-pressed` with edge `2`, which starts `Gesture::TransitionResize`; the drag updates the echo through `Studio::transition_duration()`'s clamp, and the release commits `Command::UpdateClip` via `set_clip_transition_duration()` |
| 2 | No live preview for transitions on the monitor | With a GPU the monitor already combined through the shader (`Monitor::texture_of` → `PreviewSources::composite` on the device). Without one it showed a dissolve; it now draws the manifest's `xfade` shape on the CPU |
| 3 | Export had no `xfade` path; packaged transitions exported as a dissolve without a GPU | Picture is composited frame by frame, never through an FFmpeg filtergraph, so an `xfade` filter was never the right shape of fix. `CpuCompositor::combine()` now draws the named shape (`concat-render/src/transitions.rs`), and a GPU-less export shows the cut the GPU shows, minus the shader's flourish |

### 🟡 Medium Priority

| # | Issue | Location |
|---|---|---|
| 4 | **Transition seam tooltip shows raw seconds** — should format as `"0.5s"` not `"0.5"` | `lanes.slint` tooltip text |
| 5 | **Category pill bar doesn't scroll on very narrow media panel** — content clips instead of scrolling | `media-pane.slint` `CategoryPillBar` |
| 6 | **`concat.twirl` and `concat.kaleidoscope` FFmpeg paths are simplified** — the GPU shaders are accurate but the FFmpeg fallback is a mirror approximation (documented in previous `HANDOFF.md`) | `packages/concat.twirl/`, `concat.kaleidoscope/` |
| 7 | **Audio effects batch** — 5 planned audio packages (Vinyl, Underwater, Megaphone, Stadium, Whisper) exist as stubs or are missing `fixtures.toml` | `packages/concat.vinyl/` etc. |

### 🟢 Nice-to-Have / Roadmap

| # | Feature | Notes |
|---|---|---|
| 8 | **Keyframe Graph Editor** — Fine-grained curve editing for animation params | `concat-project/src/animation.rs` + new UI |
| 9 | **Generators** (`Kind::Generator`) — Gradients, noise, solid backgrounds | Reserved in `manifest.rs`, no packages yet |
| 10 | **Effect Mixer / Transition Mixer** — Combine nodes into custom effects | Major new system; `concat-render` + new UI |
| 11 | **Concat Hub / Registry** — Community package registry | Infrastructure + distribution |
| 12 | **Object & Face Tracking** — Attach effects/text to tracked targets | `concat-vision` |

---

## 8. Verification Commands (Run After Any Change)

```bash
cd src

# All tests including GPU shader pipeline
cargo test -p concat-effects -p concat-export -p concat-render -p concat-project --features gpu

# Full app type-check (catches Slint UI macro errors too)
cargo check -p concat

# Lint
cargo clippy -p concat -p concat-export -p concat-effects -p concat-render -p concat-project \
  --features concat-render/gpu -- -D warnings

# Run the app
cargo run -p concat
# or faster:
cargo run --profile quick -p concat
```

Test count as of this handoff: **128 passing** (`concat-effects`, `concat-export`, `concat-project`). The GPU render tests (`concat-render --features gpu`) add more on top of that.

---

## 9. Git State

Everything described in §6 is **uncommitted**, sitting as working tree changes on `main`. Nothing has been pushed.

```bash
git status   # See all modified and untracked files
git diff     # See all line-level changes
```

Suggested commit strategy:
1. **Engine fix** (`concat-export` transition clamp + tests) — atomic commit
2. **UI redesign** (seam handles + animations) — atomic commit  
3. **Effects library expansion** (19 effects + 4 transitions + category pills) — could be one or split by category
4. **Click bug fix** (`auditions: false` on effects/transitions shelf) — atomic commit, clearly described

---

## 10. Critical Gotchas Summary

1. **Cargo workspace is `src/`, not repo root** — `cd src` before every `cargo` command
2. **`auditions: true` on effects/transitions shelves = global monitor overlay bug** — only Filters should use auditions
3. **Transitions have no FFmpeg filter path** — a `Kind::Transition` package is a WGSL body; `manifest.rs` validation enforces this. Without a GPU the CPU compositor draws the manifest's `xfade` shape in place of the shader (`concat-render/src/transitions.rs`); a name that file does not know, or no name, is the dissolve fallback
4. **Fixture strings are exact text diffs** — even a space change in an FFmpeg chain breaks the test; regenerate with `--nocapture`
5. **Don't use trig in `{...}` template expressions** — use FFmpeg's `geq` and always test the chain against a real `ffmpeg` binary
6. **`concat.light-leak` (effect) ≠ `concat.light-leak-transition` (transition)** — catalogue ID namespace is flat; duplicates are rejected at build time
7. **GPU regression tests are the only thing catching bind-group-layout bugs** — run `--features gpu` after any shader change
8. **Handle constraint in transitions is intentional** — you can't add a 1s transition to a clip that has 0 handles; the UX should guide the user, not silently clamp to 0
