# Concat Architecture

A study map of the system as it stands (August 2026): what each layer owns, how
they talk, where the sharp edges are, and what deserves attention next. File
references are current as of this writing.

> **September 2026:** the UI layer described below is being replaced. The
> Slint editor window now lives in the engine workspace as the `concat` crate
> (`engine/crates/concat`), ported from the wc-ui-rnd research tree, and
> `desktop/` — both the React UI and the Tauri host — is deprecated. The
> engine doctrine in §1 is unchanged and is the whole reason the swap is
> possible: the engine owns the project, the render path and the document, so
> the UI is the replaceable part. Everything from §2 on describes the Tauri
> app, which still ships until `concat` is wired to the engine. The IPC
> surface in §5 and the chains mirror in §6.1 go away with it: a Rust window
> calls the engine crates directly, in-process, and has no reason to
> re-author FFmpeg filter strings in a second language.

---

## 1. The three layers and the one rule

```mermaid
flowchart LR
    subgraph UI["desktop/src — React UI"]
        APP["App.tsx<br/>layout, selection, dialogs"]
        HOOKS["useEngineSession<br/>useEngineTruth<br/>usePlaybackBridge"]
    end
    subgraph HOST["desktop/src-tauri — Tauri host"]
        CMD["39 #[tauri::command]<br/>editor_api · playback · transcribe · templates"]
        SESSION["Session<br/>(one Mutex&lt;Option&lt;Session&gt;&gt;)"]
    end
    subgraph ENGINE["engine/crates — the product"]
        CORE[concat-core]
        MEDIA[concat-media]
        PROJECT[concat-project]
        RENDER[concat-render]
        EXPORT[concat-export]
    end
    UI -- "invoke() — JSON + ArrayBuffer" --> HOST
    HOST -- events --> UI
    HOST --> ENGINE
    ENGINE -.-> FF["ffmpeg / ffprobe<br/>subprocesses"]
```

**The rule (engine doctrine):** everything important lives in the engine
crates. The Tauri host is plumbing; the UI renders state and sends commands.
The engine owns the project model, the undo history, the render path, the
document format. When a feature is being designed, the first question is
"which engine crate does this belong to" — TS holds only what the UI
genuinely owns (layout, selection, in-flight gestures).

The engine crates keep a strict dependency arrow (see `engine/README.md`):
`core` depends on nothing, everything else depends inward, and `concat-project`
knows nothing about rendering. The desktop crate consumes the engine as path
dependencies — deliberately *not* one workspace, so the engine builds, tests
and versions alone.

| Crate | LOC | Owns |
|---|---|---|
| `concat-core` | ~1.4k | Rational time, frame model, arena handles, timeline model. Zero dependencies. |
| `concat-media` | ~3.0k | Everything FFmpeg: probe, subprocess decode/encode, reader pool with byte-budgeted LRU frame cache, waveform peaks. Optional `ffi` feature links FFmpeg for exact seeks. |
| `concat-project` | ~3.3k | The document: model, every edit command (`commands.rs`, ~1.3k), undo `Editor`, concat.json round-trip. |
| `concat-render` | ~1.3k | Compositing. `CpuCompositor` is the reference; `WgpuCompositor` behind the `gpu` feature, falls back cleanly when no GPU. |
| `concat-export` | ~2.2k | Timeline → file: flatten, filtergraph builders (`chains.rs`), the frame-by-frame render loop. |
| `concat-cli` | ~0.2k | Probe/render vertical slice for testing the engine without the app. |

---

## 2. The edit loop — how a keystroke becomes truth

The engine owns the project; the UI never mutates its own copy of the model.
Every edit round-trips:

```mermaid
sequenceDiagram
    participant UI as React component
    participant Q as useEngineSession queue
    participant H as editor_api.rs
    participant E as Editor (concat-project)

    UI->>Q: dispatch({op:"splitClips", ...})
    Note over Q: commands serialise through one<br/>promise chain, seeded with editorOpen<br/>so nothing races the session install
    Q->>H: invoke("editor_apply", command)
    H->>E: editor.apply(command)
    E-->>H: Outcome + new project state
    H-->>Q: EditorView (the whole truth)
    Q->>UI: setView(next) — UI re-renders from engine state
```

Key properties:

- **One queue.** Commands serialise so responses can never land out of order
  and overwrite newer state with older (`useEngineSession.ts`).
- **The echo layer.** Drags don't send a command per pixel. The UI previews
  the gesture locally ("echo"), using arithmetic mirrored from the engine,
  and commits **one** command per gesture on release — so undo undoes the
  drag, not a pixel of it. The echo clears only after the engine's state
  arrives, so nothing flashes back mid-flight.
- **Undo lives in the engine** — a bounded history of full `Project` clones
  (depth 200), not command inversion.
- **Autosave** debounces 1.5 s after any state change; the engine writes the
  document (temp file + rename, so a crash mid-save can't truncate the
  project).
- **Wire types are generated.** `EditorView`, `Command` and friends are
  ts-rs exports from the Rust types into `desktop/src/lib/generated/`;
  CI fails if the committed files drift from the Rust source of truth.

---

## 3. Preview & playback — two clocks, one truth

Audio and video take different paths, deliberately (desktop decisions 0005,
0009 — see §7):

```mermaid
flowchart TB
    subgraph AUDIO["Audio — engine-owned (playback.rs)"]
        DEC["decode worker pool<br/>clip span → WAV in cache/audio"]
        MMAP["mmap'd PCM"]
        CPAL["one cpal callback mixes everything"]
        CLK["device sample counter →<br/>AtomicU64 position_micros"]
        DEC --> MMAP --> CPAL --> CLK
    end
    subgraph VIDEO["Video — pulled by the UI"]
        TRUTH["useEngineTruth pull loop<br/>one request in flight, snapped to frame grid,<br/>decode-ahead clamped to 1–3 frames"]
        PF["preview_frame → raw RGBA<br/>(tauri ipc::Response = ArrayBuffer, no JSON)"]
        CANVAS["putImageData on the monitor canvas"]
        TRUTH --> PF --> CANVAS
    end
    CLK -- "transport events" --> TRUTH
```

- **The audio device is the only clock.** The transport thread publishes
  position; the UI interpolates between events. Video chases audio, never
  the other way around.
- **The monitor draws the engine's real composited frame** — the same
  pixels the exporter would produce — at `previewQuality` (default 0.5)
  to keep the per-frame RGBA copy over IPC affordable.
- **Effects preview in two grades:** live approximations (CSS filters /
  canvas ops in `lib/effects.ts`) during interaction, engine truth for the
  monitor. The export is "only ever a higher-quality version of what you
  already saw."
- All 15 byte-shipping commands use `tauri::ipc::Response` (ArrayBuffer),
  so no base64 or number-array serialisation on the hot paths. The one
  exception: `write_artwork(bytes: Vec<u8>)` serialises inbound as a JSON
  number array.

---

## 4. Export

Hybrid pipeline in `concat-export`:

- **Picture:** flattened timeline → per-frame composition in-process
  (CPU or wgpu) → raw frames piped into one `ffmpeg` encode subprocess.
- **Audio:** one FFmpeg filtergraph mixes every audible clip in a single
  pass.
- **Transitions are lowered before either path runs** — a cross-fade
  becomes overlapping clips with opacity ramps on doubled track indices,
  so the compositor never knows transitions exist.
- Filter strings are byte-pinned by tests on **both** sides of the mirror
  (see §6, the chains mirror).

---

## 5. IPC surface (39 commands)

| Group | Commands |
|---|---|
| Editor session | `editor_open/apply/undo/redo/save/state/close` |
| Projects & recents | `create_project`, `open_project`, `recent_projects`, `forget_project`, `project_preview` |
| Media | `probe_media`, `read_media_bytes` (capped), `extract_peaks`, `extract_filmstrip`, `read_artwork`, `write_artwork` |
| Preview & transport | `preview_frame`, `preview_prefetch`, `transport_play/pause/seek`, `audio_set_clips` |
| Export | `export_project`, `cancel_export` |
| Templates | 5 commands |
| Transcription | 7 commands |

---

## 6. Footguns and technical debt

The honest list, ordered by how much they matter. Nothing here is on fire;
several are traps waiting for a contributor who doesn't know the invariant.

### 6.1 The chains mirror (highest-leverage debt)

`desktop/src/lib/effects.ts` + `filters.ts` are **hand-maintained TS mirrors**
of `engine/crates/concat-export/src/chains.rs`. Both sides emit FFmpeg filter
strings; the only thing keeping them identical is a corpus of pinned-string
tests on each side (`chains.rs:523+` vs `effects.test.ts` / `filters.test.ts`).
This is the largest deliberate violation of "the engine owns it": the TS side
exists so previews can be computed without IPC, but every new effect must be
written twice and pinned twice. It also means `usePlaybackBridge` builds a
filter string *in TS* and ships it to Rust — the UI is composing engine
commands in the engine's own language. Worth studying: could the engine emit
the preview approximation (CSS filter string, canvas op list) alongside the
chain, making TS a renderer of engine-provided looks instead of a second
author?

### 6.2 Lock discipline in `playback.rs`

`playback.rs` uses `.lock().unwrap()` on shared state ~14 times across four
kinds of threads (mix callback, transport, decode workers, cache sweep). One
panic in any decode worker poisons the mutex and cascades unwraps through the
others. Meanwhile `editor_api.rs` converts poison to a `String` error at
`:120,188` but silently ignores it at `:266` — three different poison policies
in one codebase. Pick one (probably: poison = clear the poisoned state and
carry on, since every cache here is rebuildable).

### 6.3 Unbounded growth

- `redo: Vec<Project>` in the engine editor has **no cap** (undo is capped at
  200) — each entry is a deep clone of the whole project
  (`concat-project/src/editor.rs:28`).
- `desktop/src/lib/assets.ts` — the `peaks`/`strips`/`stripFrames` maps grow
  per media id with no eviction, and no `ImageBitmap.close()` is ever called;
  GPU-backed bitmaps wait for GC. A long session with many imports leaks
  VRAM-adjacent memory.

### 6.4 Swallowed errors

Twenty `catch(() => undefined)`-shaped sites. Most are defensible
(fire-and-forget artwork writes), but four are in the engine-session queue
itself (`useEngineSession.ts:101,118,129,206`) — a failed **undo/redo**
drains silently. Host side: `export.rs:38` drops progress-event send
failures; `playback.rs:251` drops audio-error emission failures. A grep for
`catch(() =>` is a good periodic audit.

### 6.5 Trust boundaries

- `Preview.tsx:869` feeds `preview_frame` bytes straight into
  `new ImageData(bytes, w, h)` — a short buffer throws inside the draw loop
  rather than failing gracefully.
- Counterexample done right: `peaks_bytes` validates cached bytes with
  `plausible_peaks` before trusting them (`lib.rs:233`).

### 6.6 Default builds don't test the real paths

`cargo test` at default features exercises **neither** the wgpu compositor
(`gpu` off) **nor** the FFI decoder (`ffi` off) — the two code paths preview
and scrubbing actually rely on in a bundled build. CI should run at least one
job with `--features gpu,ffi`.

### 6.7 Frontend hotspots

- `App.tsx` holds **30 `useState`s**; `<Preview>` takes ~46 props,
  `<TimelinePanel>` ~39, `<MediaBin>` ~28. The hooks
  (`useEngineSession/Truth/PlaybackBridge`) are the right pattern — more of
  App.tsx's state wants to migrate into them (selection + tool state is the
  obvious next hook).
- The five biggest files: `TimelinePanel.tsx` 1786, `App.tsx` 1583,
  `Preview.tsx` 1429, `MediaBin.tsx` 1072, `lib/effects.ts` 694.
- `lib/monitor.ts` re-derives "what is on screen / what exports" in parallel
  with the engine's flattener — same mirror concern as §6.1 in miniature.

### 6.8 Panics in host runtime paths

Thread spawns `.expect()` in `playback.rs:271,289,314,442`;
`editor_api.rs:190` `.expect("just set")`. Engine `.expect()`s exist in
`decode.rs`, `encode.rs`, `pool.rs`, `time.rs:97` ("rational overflowed
i64") — most are genuine invariants, but each is a crash-not-error if wrong.

### 6.9 Housekeeping

- **Zero TODO/FIXME/HACK comments** in the entire codebase — invariants live
  in prose comments instead. Keep it that way.
- `desktop/src-tauri/src/export.rs` is a 42-line shim left over from moving
  export into the engine; fold it into `lib.rs` or leave it as the seam.

---

## 7. The lost decision records

Code comments still cite `docs/decisions/…` paths that were deleted in commit
`36168ac` ("docs: drop the decision logs"). Four ids are referenced from
surviving code (`ffi.rs:9`, `gpu.rs:4`, `playback.rs:3`). The titles, for
recovery via `git show` if wanted:

- *engine:* 0002 ffmpeg-over-a-pipe · 0003 arena-handles-not-pointers ·
  0004 cpu-compositor-first · 0005 license-mpl-2 · 0006
  transitions-in-the-exporter · 0007 engine-owns-the-project · 0008
  templates-are-documents-plus-slots · 0009 the-engine-owns-the-render-path
- *desktop:* 0001 tauri-over-electron · 0002 the-hot-surfaces-are-not-the-dom
  · 0003 csp · 0004 audio-preview-in-the-webview (superseded) · 0005
  audio-playback-in-the-engine · 0006 multiple-timelines · 0007
  preview-approximation-layer · 0008 bundled-ffmpeg-and-releases · 0009
  the-monitor-streams-the-true-frame · 0010 the-app-installs-its-own-tools

Either restore them or strip the dangling references — a comment pointing at
a deleted document is worse than no comment.

---

## 8. Test coverage map

| Area | Tests | Gaps |
|---|---|---|
| Engine crates | 179 (core 31, media 48, project 41, render 28, export 32, cli 3) | `gpu`/`ffi` features untested at defaults (§6.6) |
| Desktop Rust | 15 | **Zero** for `lib.rs` (all 25 commands), `editor_api.rs` — the session lifecycle that produced three of this week's bug reports |
| Frontend lib | 98 across 9 files | solid |
| Frontend components/hooks | **0** | `useEngineSession` (queue, echo, autosave) is the highest-value target; the big components need at least smoke renders |

---

## 9. Where to improve first (opinionated)

1. **Session-lifecycle tests for `editor_api.rs`** — open/apply/close against
   temp project folders. The "fresh project unopenable" and "import before
   open" bugs both lived here; both were testable without a UI.
2. **One poison policy** in `playback.rs` (§6.2) — mechanical, removes the
   cascade failure mode.
3. **Cap the redo stack and evict `assets.ts` bitmaps** (§6.3) — small
   patches, close real leaks.
4. **Surface the swallowed undo/redo failures** (§6.4) — they already have a
   toast pathway.
5. **A `--features gpu,ffi` CI job** (§6.6).
6. **The chains mirror** (§6.1) — the big one; a design conversation, not a
   patch. Engine-emitted preview looks would delete ~700 lines of mirrored
   TS and the entire pinned-string coupling.
7. **Decompose App.tsx state into hooks** (§6.7) — as touched, not as a
   rewrite.
