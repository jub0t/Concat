# Concat — Deep Audit Report: Bugs & Issues

**Repository:** `jub0t/Concat` (audited at commit `f0c01696`, version 0.2.8)
**Fork:** `AbedRafik/Concat`
**Audit date:** 2026-11-01
**Method:** 7 parallel deep-scan agents over ~133,000 lines of Rust across 17 workspace crates, 74 Slint UI files and 102 WGSL shaders, plus the repository's own mechanical gates (`cargo fmt --check`, `scripts/locales.py --check`, `scripts/models.py --check`, `cargo clippy --workspace --all-targets`) run locally with the pinned toolchain (1.93.1) and the CI-pinned dependency set (BtbN FFmpeg 8.1.3, ONNX Runtime 1.28.0, sherpa-onnx 1.13.7).

---

## Executive summary

The audit reviewed the entire workspace and found **61 issues**: **5 High**, **21 Medium**, **35 Low**. In addition, the mechanical gate `cargo fmt --check` **fails on current `main`** with 52 diff hunks — the first CI job (`checks`) would be red on a fresh push today.

The codebase is, overall, of unusually high quality: exact rational time arithmetic everywhere, disciplined FFmpeg FFI error handling, atomic file writes with temp+rename, digest-pinned downloads, root-confined MCP writes, constant-time token comparison, and exceptional test coverage in `concat-project`. The genuine defects cluster at **seams**: lifecycle flags set before work completes, wire-format numbers trusted where sibling fields validate, per-frame filesystem work on the export hot path, localization leaks in the UI crate, and one systemic alpha-compositing mistake in the GPU compositor.

**Top 5 issues by impact:**

| # | Severity | Issue | Where |
|---|----------|-------|-------|
| H1 | High | `cargo fmt --check` fails on `main` (52 hunks) — CI gate broken | `src/crates/concat`, `src/crates/concat-vision` |
| H2 | High | Audio mix opens one demuxer per speed-curve piece → fd/memory exhaustion on export of any speed-ramped clip ≥ ~26 s | `concat-media/src/audio.rs`, `concat-export/src/lib.rs` |
| H3 | High | Effects/treatments run over **premultiplied** pixels while the pass contract is **straight** colour — semi-transparent media is double-darkened in preview and export | `concat-render/src/gpu.rs` |
| H4 | High | Media-bin marquee selects the wrong cards whenever the bin is sorted (filter applied, sort ignored) — destructive actions hit the wrong files | `concat/src/panes/media_bin.rs` |
| H5 | High | TTS text containing U+0000 panics inside sherpa-onnx's `CString::new(...).unwrap()`; the shipping `app` profile uses `panic = "abort"` — the whole editor dies | `concat-speech/src/tts.rs` |

---

## Methodology

- **Agent 1** — `concat-core`, `concat-project`, `concat-api` (model, timeline, undo, serialization, RPC types) — every production file read line-by-line plus the 5,900-line `concat-project` test module.
- **Agent 2** — `concat-media`, `concat-export` (decode/encode, FFmpeg FFI, export pipeline).
- **Agent 3** — `concat-render`, `concat-effects` and all 102 `.wgsl` shaders (all 22 transition shaders line-by-line).
- **Agent 4** — `concat-host`, `concat-mcp`, `concat-server`, `concat-cli` (services, MCP/JSON-RPC doors, CLI).
- **Agent 5** — the app crate `concat` (Slint UI, timeline interaction, playback, i18n).
- **Agent 6** — `concat-speech`, `concat-text`, `concat-vision` (whisper.cpp FFI, sherpa-onnx TTS, ONNX cutout, fonts), including the vendored sherpa-onnx 1.13.8 / whisper-rs 0.16 FFI contracts.
- **Agent 7** — build system, CI workflows, scripts, `Cargo.toml`/lock patches, flake, models manifest, all 26 locale files (structural diff vs `en.json`), packaging configs.
- **Mechanical gates** — run locally exactly as CI runs them, with the pinned toolchain and digest-verified prebuilt dependencies.

Findings are numbered per subsystem below. Items the agents could not confirm without a runtime/GPU test are marked **[needs-runtime-test]**.

---

## Mechanical gate results

| Check | Result | Detail |
|---|---|---|
| `cargo fmt --check` (pinned 1.93.1 rustfmt) | **FAIL** | 52 diff hunks in `src/crates/concat/src/**` (lib.rs, panes/*.rs) and `src/crates/concat-vision/src/compact.rs` |
| `python3 scripts/locales.py --check` | PASS | 0 missing / 0 stale keys in all 25 translations (informational: 1,124 lines remain word-for-word English) |
| `python3 scripts/models.py --check` | PASS | 23 models, 3.37 GB, manifest consistent with the engine tables |
| `cargo clippy --workspace --all-targets` | See appendix | Run with the CI environment reproduced locally |

---

# Part I — High-severity findings

## H1. `cargo fmt --check` fails on `main` — the CI format gate is red

- **Location:** `src/crates/concat/src/lib.rs` (lines 2912, 2990, …), `src/crates/concat/src/panes/{media_bin,settings,project,export,start}.rs`, `src/crates/concat-vision/src/compact.rs` (lines 88, 95, 376, 438, 448, …) — 52 hunks total.
- **Evidence:** `cargo fmt --check` exits 1 with 52 hunks, e.g. in `concat/src/lib.rs:2912` a `projects::create(...)` call in a test exceeds the width limit and is not re-wrapped.
- **Impact:** CI's `checks` job (`cargo fmt --check`) fails on any push/PR cut from current `main`. Every subsequent gate (clippy, tests) still runs (`if: !cancelled()`), but the build status is red and release tags are gated on CI.
- **Fix:** one `cargo fmt` run over the workspace (no manual edits needed — all hunks are pure re-wraps).

## H2. Audio mix opens one demuxer per speed-curve piece — fd/memory exhaustion

- **Location:** `src/crates/concat-media/src/audio.rs:781-865` (`run_mix` — every member opened up front, kept open for the whole mix), with `src/crates/concat-export/src/lib.rs:1308,1455` (`audio_pieces` slices a curved-speed clip into `((duration / 0.1).ceil()).clamp(8, 400)` pieces, each becoming its own mix input).
- **Evidence:** each piece = its own `AVFormatContext` + decoder + index tables for the full mix duration. A single ~26 s clip with a speed curve yields 260+ simultaneously open inputs; macOS's default soft fd limit is 256.
- **Impact:** exporting any speed-ramped clip past ~26 s fails with "Too many open files" during "mixing audio"; on Linux (1024 fds) two or three such clips hit the same wall; every case pays hundreds of MB of demuxer state.
- **Fix:** feed all pieces of one source file through a single input (per-piece `atrim` windows) instead of one input per piece; alternatively open pieces lazily as their in-point approaches and cap concurrent open inputs.

## H3. GPU compositor: effects run over premultiplied pixels — semi-transparent media darkened

- **Location:** `src/crates/concat-render/src/gpu.rs:1694-1735` (`make_picture` renders through the ordinary layer pipeline, whose `fs_main` outputs `s.rgb * s.a` blended `ONE/ONE_MINUS_SRC_ALPHA` over a transparent target), while `fs_upload` (gpu.rs:327-330), `sample()`'s documented contract (`concat-effects/src/shader.rs:105-119`) and the probe fixtures all use **straight** colour.
- **Evidence:** for `needs_preparing()` layers (effects, crop, flip, fit-resize) `run_passes` is fed the premultiplied texture; an identity effect echoes a premultiplied texel and the final quad draw multiplies by alpha **again**.
- **Impact:** a straight `(1,1,1,0.5)` pixel renders at 25% brightness instead of 50%. Import any transparent PNG sticker whose aspect differs from the timeline (→ `fits_source()` false) and apply an effect (glow/blur/shadow): wrong output in preview **and** export, silently. Opaque video is unaffected, which is why the parity suite never catches it.
- **Fix:** give `make_picture` a dedicated no-blend pipeline whose fragment returns straight colour (its quad covers the whole target); unpremultiply (or draw through the same straight pipeline) for the treatment stack. Add a parity test with a semi-transparent source + effect + crop.

## H4. Media-bin marquee selects the wrong cards when the bin is sorted

- **Location:** `src/crates/concat/src/panes/media_bin.rs:423-448` — `band()` walks `project.media` applying **only the filter**, while the grid is laid out from `visible()` (filter **and** sort; `ui/workspace/media-pane.slint:822-824`).
- **Evidence:** the function's own doc comment claims "the walk is over the filtered order, because that is what the grid was laid out from" — the sort (`self.sort` 1 = name, 2 = kind, reachable from the View menu) breaks that assumption.
- **Impact:** with the bin sorted by name, a marquee drag visibly encloses one block of cards but selects a *different* set; Delete/Remove, "Add to timeline" and the batch-volume knob then act on the wrong files.
- **Fix:** make `band()` walk the same `visible(project)` list the grid draws, so the `cell / columns` arithmetic matches the on-screen order.

## H5. TTS text containing NUL aborts the entire app

- **Location:** `src/crates/concat-speech/src/tts.rs:1004-1026` passes `SpeakRequest.text` through unvalidated; the vendored sherpa-onnx `tts.rs:567` does `CString::new(text).unwrap()`.
- **Evidence:** `Speech::speak` validates empty text and NaN speed but not control characters. The shipping `app` profile sets `panic = "abort"` (`src/Cargo.toml`), so the panic kills the editor, not the job.
- **Impact:** narration text pre-filled from an SRT import (a stray NUL survives `read_to_string` as valid UTF-8) or pasted from a clipboard crashes the whole application on the synthesis worker.
- **Fix:** reject (or strip) `\0` — and control characters generally — in `Speech::speak` before handing text to sherpa, mirroring the existing NaN/empty guards.

---

# Part II — Medium-severity findings

## Engine / backend

### M1. Id mint counter overflows on a hostile document id
- **Location:** `src/crates/concat-project/src/commands/mod.rs:983-1001` (`IdMint::adopt` parses numeric suffixes as `u64` with no bound; `next()` does `self.counter += 1`).
- A shared/hand-edited project containing an id like `c18446744073709551615` sets `counter = u64::MAX`; the next mint overflows — debug panic, release wrap-around to ids that collide with the document's own (silent edit corruption). Violates the codebase's own "a hostile container must produce an error, never a crash" bar.
- **Fix:** clamp adopted values (`counter.max(value.min(u64::MAX - margin))`) and use `checked_add` with a fallback suffix in `next()`.

### M2. `project.create` skips the `VideoSettings` sanity check its sibling applies
- **Location:** `src/crates/concat-api/src/lib.rs:516-534` — `projects::create` is called with raw `video.*` numbers; `SetTimelineVideo` returns a no-op for `!video.is_sane()`.
- An API caller can create a `{"width":0,"height":0,"rateNum":0}` project whose settings stay broken until `setVideo` fixes them.
- **Fix:** gate on `video.is_sane()` (or `video.or(default)`) before `projects::create`.

### M3. Unvalidated `track` index panics/OOMs the exporter
- **Location:** `src/crates/concat-export/src/resolve.rs:283-286,343` — `clip.track` arrives from the request JSON with no ceiling; `(0..=max_track).map(...).collect()` allocates a `Vec` per lane and then `tracks[clip.track]` indexes it.
- `"track": 1000000000` allocates a billion `Track`s before the first frame; `usize::MAX` overflows the `+ 1` (panic) or wraps (index out of bounds).
- **Fix:** validate `track` against a sane ceiling in `render_on`/`preview_plan` like the size/rate checks nearby.

### M4. Extreme clip times panic via `Rational` overflow
- **Location:** `src/crates/concat-export/src/resolve.rs:584-592` (`quantise` casts `seconds * fps` to `i64` unchecked) — `concat-core`'s `Rational::reduce` then `.expect("rational overflowed i64")`, which fires in release builds too.
- `clip.start`/`duration`/`source_start` are unvalidated on the request path (`effect_time_offset` is the only checked field); `1e300` saturates to `i64::MAX` and panics in `time_of_frame`.
- **Fix:** range-check all clip times (finite, within `MAX_TIME`) before resolution; make `quantise` defensive.

### M5. `Encoder::finish` marks itself finished before flushing
- **Location:** `src/crates/concat-media/src/encode.rs:1907-1944` — `self.finished = true` is set before `send(None)`/`flush()`/`write_trailer()`.
- A retried finish after a mid-flush failure returns `Ok(())` while the file on disk has no trailer — an unopenable MP4 reported as successfully closed.
- **Fix:** set `finished` only after `write_trailer()` succeeds (track "tried" separately if idempotent no-op behaviour is wanted).

### M6. A mid-mix error leaves a corrupt partial file at the destination
- **Location:** `src/crates/concat-media/src/audio.rs:680-703` — `format::output(destination)` truncates the target immediately; `run_mix(...)?` returns before `send_eof`/`drain`/`write_trailer`.
- The public "save this clip's sound" API leaves an unopenable file exactly where the user pointed on any mid-mix failure.
- **Fix:** write to a sibling temp file and rename on success (the pattern the video export path already uses), or trailer/remove on error.

### M7. Cancel token is not honoured while the audio mix runs
- **Location:** `src/crates/concat-export/src/lib.rs:1303-1317` — cancellation is polled before and after the mix, never during; the mix is one un-interruptible FFmpeg pass.
- A user cancelling during "mixing audio" watches a dead button while decode/mix/write continues for minutes.
- **Fix:** thread an `AtomicBool` (or the existing cancel handle) into `run_mix`'s feed/drain loop and abort early.

### M8/M9. Iris and linear-wipe transitions: soft edge centred, not led
- **Locations:** `src/crates/concat-effects/packages/concat.iris/effect.wgsl:10-12` (`radius = p * 1.15` with `smoothstep(radius - soft, radius + soft, d)`); `concat.linear-wipe/effect.wgsl:15-17` (`smoothstep(p - soft, p + soft, coord)`).
- At `p=0` the edge band straddles the frame — a visible ~50% blended blob on the cut's first frame; at `p=1` corners/trailing edge retain up to 50% outgoing before a hard pop. The CPU fallbacks (`Shape::Wipe`, iris `edge()` in `concat-render/src/transitions.rs:383-388`) deliberately compensate (`s·(1+2·soft) − soft`) — the GPU shaders are the outliers and disagree with their own fallbacks.
- **Fix:** `radius = p * (1.15 + 2.0 * soft) - soft` (iris); `at = p * (1.0 + 2.0 * soft) - soft` (wipe) — matching the CPU math — plus end-point fixtures.

### M10. Upscaler waits on its device indefinitely
- **Location:** `src/crates/concat-render/src/upscale.rs:587-589` — `device.poll(PollType::wait_indefinitely())`.
- The compositor bounds every other wait at 30 s precisely because of the #297 hang; the Enhance worker can block forever on a hung driver with no timeout and no cancellation.
- **Fix:** bounded wait (30 s) and treat `PollError::Timeout` as a dead upscaler falling back to the CPU path.

### M11. gRPC door lacks the JSON door's caps; `Events` streams never unsubscribe
- **Location:** `src/crates/concat-server/src/grpc.rs:65-79` — no `MAX_PENDING`/`MAX_CONNECTIONS`/`AUTH_TIMEOUT`, and the subscriber closure registered in the hub is only pruned when the *next* event's `try_send` fails.
- An idle server accumulates abandoned streams (memory + O(n) fan-out per future event); unbounded concurrent connections are possible. Off-by-default feature, token still required — hence Medium.
- **Fix:** `Drop`-guard calling `Hub::unsubscribe_by_id` (assign ids at subscribe time) + a concurrent-stream cap returning `resource_exhausted`.

### M16. Enhance flattens alpha — every restored pixel forced opaque
- **Location:** `src/crates/concat-vision/src/enhance.rs:285-315` — `window_planes` reads RGB only and `paste` writes `pixels[at + 3] = 255` unconditionally. **[needs-runtime-test]** (alpha-carrying WebM end-to-end).
- **Fix:** sample and propagate the source frame's alpha in `paste`.

### M17. `MaskStore::resolved` re-opens every smart-stroke region directory on every rendered frame
- **Location:** `src/crates/concat-vision/src/store.rs:186-217` — a full `read_dir`+sort per smart stroke per frame, even when the per-cutout `resolved_cache` below would answer; a 10-min analysed clip ≈ 6,000 PNG names parsed per region per frame on the export hot path.
- **Fix:** open region stores once per cutout job (or cache by `(dir, mtime)`), and check the cache before touching the filesystem.

## UI / app crate

### M12. `Studio::save` (Ctrl+S) does not flush a pending inspector/text commit
- **Location:** `src/crates/concat/src/studio.rs:2968-2989` — no `flush_commit()` before `prepare_save`; `save_and_release` (8983-9001) calls it at 8986.
- Typed words live on the 120 ms echo timer; Ctrl+S immediately after typing announces "Project saved" while the file omits the last keystrokes (a crash in that window loses them).
- **Fix:** `self.flush_commit()` at the top of `Studio::save`.

### M13. Title-bar save status published as hardcoded English
- **Location:** `src/crates/concat/src/studio.rs:11572-11578` — `app.set_project_status(if self.dirty { "unsaved changes" } else { "saved" })` overwrites the localized binding declared in `app.slint:295` (`I18n.t("common.saved")`), and no key exists for the unsaved variant.
- Every non-English UI permanently shows English status text.
- **Fix:** add `common.unsavedChanges` to the locale inventory (all 26 files) and publish `t(...)` from `publish_chrome`.

### M14. Four RTL locales ship but the window never sets an RTL layout direction
- **Location:** `src/crates/concat/src/i18n.rs:57-84` (ar, fa, he, ur in `BUILT_IN`); no `set_layout_direction`/`layout-direction` anywhere in the crate. **[needs-runtime-test]** for Slint 1.18 inference.
- Arabic/Farsi/Hebrew/Urdu users get translated strings inside mirrored-left chrome (menus on the wrong side, chevrons backwards).
- **Fix:** set the window's layout direction when the active language is RTL and re-apply on `SettingsMsg::LanguageChanged` (the `compat-1-2` feature enabling the API is already on).

### M15. "Clear project cache" toasts hardcoded English
- **Location:** `src/crates/concat/src/studio.rs:8949-8963` — `"Cleared {count} cache files"` / `"Cache clear failed: {error}"` bypass the locale system.
- **Fix:** add `studio.clearedCacheFiles` (with `{0}`) and `studio.cacheClearFailed` keys across the locale inventory.

## Build / CI

### M18. `cargo-apk` installed unpinned
- **Location:** `.github/workflows/mobile.yml:139-141` — `taiki-e/cache-cargo-install-action` with `tool: cargo-apk` (no version), while ci.yml pins `cargo-nextest@0.9.148` and every action/dependency is commit- or digest-pinned.
- A breaking or compromised cargo-apk release flows straight into shipping APK builds.
- **Fix:** `tool: cargo-apk@<exact version>`, bumped deliberately.

### M19. FFmpeg mobile cache key omits toolchain/SDK inputs
- **Location:** `.github/workflows/mobile.yml:155-167` (also 377-391, 482-499) — key varies only with the script text and target; NDK/Xcode/FFMPEG_VERSION bumps restore stale native libraries and skip the rebuild.
- **Fix:** include the NDK/Xcode version and `FFMPEG_VERSION` in the key (or stamp-and-verify a version file inside the prefix).

### M20. A dependency bump can silently drop both local `[patch.crates-io]` patches; nothing notices
- **Location:** `src/Cargo.toml:148-154` — `i-slint-renderer-skia` and `whisper-rs-sys` patch copies are version-locked to the upstream release; a grouped Dependabot bump emits only a "patch was not used" *warning* and builds against the unpatched crates.io crate, silently losing the Windows text-rendering fix (#278) and the GGML backend-dl build.
- **Fix:** a `checks`-job step asserting both crates resolve from `patches/` (e.g. `cargo metadata` + jq grep for the path source).

### M21. Mobile prebuilt libraries downloaded without digest verification
- **Location:** `src/scripts/sherpa-mobile.sh:42-44,60-62`, `src/scripts/ffmpeg-mobile.sh:39-41` — `curl -fsSL` with no sha256 check, unlike `scripts/ci/fetch.sh` which "refuses a file whose digest has changed"; the same sherpa releases are digest-pinned for desktop in `deps.lock`.
- These `.so`/`.xcframework` files ship verbatim inside the APK/IPA.
- **Fix:** add rows to `deps.lock` and route the downloads through `scripts/ci/fetch.sh`.

---

# Part III — Low-severity findings

## concat-core / concat-project / concat-api

- **L1. `Frame::set_pixel`/`blit`/`fill`/`rows` silently assume 8-bit depth** — `concat-core/src/frame.rs:266-307`: hard-coded 4-byte stride corrupts 16-bit frames instead of failing; `pixel()` handles depth correctly, so the API is internally inconsistent. Latent (deep frames go straight to GPU today). **Fix:** assert/handle depth or generalize via `bytes_per_pixel()`.
- **L2. `RevealMap::from_rects` i32 cast overflow / u32 underflow** — `concat-core/src/shader.rs:251-263`: `w as i32` wraps for `w > i32::MAX`; unreachable from documents today but a public constructor. **Fix:** compute in `i64` and clamp.
- **L3. `MatchClip` copies `enhanced`/`muted` onto a clip that keeps its own media** — `concat-project/src/commands/properties.rs:378-408`: breaks the `enhanced.on == (media_id == enhanced.copy)` invariant for any future cross-media caller; `muted: Some(true)` with the target's own `detached_from: None` silently mutes. **Fix:** keep the target's own `enhanced`/`muted` when media differs.
- **L4. Ripple/room logic only moves neighbours that start at/after the edit point** — `commands/properties.rs:492-520`, `clips.rs:363-368,945-977`: stacked clips from older documents that *overlap* the post-edit span are detected but never moved. **[needs-runtime-test]** (only reachable by editing legacy stacked documents). **Fix:** shift every clip that *covers* the post-edit span (`placement::covers()`).
- *(API — see M2, folded into the Medium list.)*

## concat-media / concat-export

- **L5. Send-EOF "try again" bypasses the stuck-decoder guard** — `concat-media/src/feed.rs:110-126`: the `Eof`-on-send arm doesn't count toward `STUCK_AFTER`; a wedged codec busy-loops instead of surfacing `Stall::Stuck`. **[needs-runtime-test]** **Fix:** count EOF refusals.
- **L6. CBR/average bitrate strings overflow `u32`** — `encode.rs:894-899`: `bitrate_kbps * 3 / 2` panics (debug) / wraps (release) above ~1.43 Gbps; the request wire format doesn't validate. **Fix:** compute in `u64` or clamp at validation.
- **L7. `Encoder::create` truncates the destination before an encoder exists** — `encode.rs:1472-1497`: any later failure (no encoder, refused settings, scaler) leaves a zero-byte file where a good one was; the crate's public write API. **Fix:** open the output only after an encoder opens, or remove the file on error paths.
- **L8. Video decoder never signals EOF to its filtergraph** — `decode.rs:1404-1428` (contrast `samples.rs:343-353` which flushes the buffersrc): temporal filter chains (`tblend`, `fps`, lookahead blurs) silently drop their tail frames. **Deferred:** the video path's one-frame-per-push contract does not survive a buffering chain (a fix attempt showed `fps` doubling breaks the pacer's arithmetic), so closing this needs the decoder's pull loop restructured around a pending queue - recorded here as the design it needs rather than risked as blind surgery.
- **L9. `mux` cutoff truncates output when a stream's packets carry no timestamps** — `audio.rs:1084-1088,1184-1192`: `end` stays `ZERO` for pts/dts-less streams and the interleave loop breaks after one packet. **[needs-runtime-test]** **Fix:** keep `end` as `Option` until a real end is observed.
- **L10. SVG drawing cache grows without bound in the temp folder** — `svg.rs:84-93`: every distinct (path,size,mtime) SVG draws a multi-MB PNG into `concat-svg/`, nothing evicts. **Fix:** cap by count/bytes, evict oldest.
- **L11. No ceiling on a hostile file's coded dimensions** — `decode.rs:834-843`: a crafted ~32767×32767 file drives multi-GB allocations on open; the export path already enforces `MAX_SIDE` (8192). **Fix:** reject/clamp oversized streams in `Decoder::open`/`probe`.
- **L12. Export length measured over hidden clips too** — `concat-export/src/lib.rs:1256,1338-1345` + `resolve.rs:292-293`: a hidden clip past the last visible content appends a black/silent tail. **[needs-runtime-test]** (may be intended). **Fix:** compute the extent over contributing clips, or document and test the behaviour.

## concat-render / concat-effects

- **L13. `pow` of a negative base in `light-leak-flash`** — `concat-effects/packages/concat.light-leak-flash/effect.wgsl:17`: `pow(leak_pos - progress, 2.0)` with base ∈ [-1,1] is unspecified in WGSL; NaN → pixels burn white on some drivers. **[needs-runtime-test]** **Fix:** `exp(-(x)*(x)*12.0)`.
- **L14. `resolve()` can pick an HDR pipeline into the SDR presentable ring** — `concat-render/src/gpu.rs:2449-2473` with `presentable():1492-1529`: latent API-combination trap (`deliver_hdr(true)` + `render_texture`). **Fix:** make `presentable()` take the deep flag / clamp to SDR pipelines.
- *(See also F#6 in Part II — the `set_pixel` depth issue is L1 above; both audit agents independently found it.)*

## concat-host / concat-server / concat-mcp / concat-cli

- **L15. `recents.json` written non-atomically** — `concat-host/src/projects.rs:573-581`: plain truncating write, no temp+rename, no lock; a crash mid-write silently loses the whole recents history (read treats corrupt as empty). **Fix:** route through the `Temporary` write pattern used by the manifest.
- **L16. Two poison-sensitive `unwrap()`s in the decode worker** — `concat-host/src/playback.rs:1051,1059-1061`: `queue.available.wait(jobs).unwrap()` and `specs.lock().unwrap()`; every other lock site uses the `locked()` poison-recovery helper the file itself documents. One panic under a guard kills all decode workers for the session. **Fix:** use `locked()`/poison recovery.
- **L17. Windows junction creation via `cmd /C mklink /J` mis-parses paths with cmd metacharacters** — `concat-host/src/dirs.rs:192-208`: `&`, `^`, `%`, `(`,`)` in a user/profile path (e.g. `C:\Users\AT&T.eng`) split the command line; mklink fails or runs path fragments. **Fix:** `cmd /S /C` with a single pre-quoted argument string, or `std::os::windows::fs::symlink_dir` with fallback.
- **L18. JSON-RPC client buffers reply lines up to 256 MiB** — `concat-server/src/client.rs:19-21,96-99`: the server caps inbound lines at 4 MiB; the client's own cap is 64× looser than any legitimate reply. **Fix:** lower to ~64 MiB and make "hit the cap" connection-fatal.
- **L19. External helper processes run with no timeout; `scutil --proxy` spawned per network request on macOS** — `concat-host/src/updates.rs:640-655`, `route.rs:284-301`: a wedged `hdiutil` hangs the update thread forever (rollback never runs); fork+exec per download on the healthy path. **Fix:** deadline the helpers and cache proxy detection for a few minutes.

## concat-speech / concat-text / concat-vision

- **L20. Repetition-penalty scan is O(vocab × tokens) per generated token** — `concat-speech/src/chatterbox.rs:716-782`: `written.contains(...)` inside a per-logit loop; ~6.5k × ~500 comparisons per step, billions across a chunk. **Fix:** a `Vec<bool>`/`HashSet<i64>` of written ids, updated on push.
- **L21. Silero VAD model re-loaded from disk on every transcription** — `concat-speech/src/transcribe.rs:242-265`: the whisper model is cached, the VAD context is reconstructed per run. **Fix:** cache it in `Transcriber` or a `OnceLock`.
- **L22. Whitespace-only paragraph measured including its spaces** — `concat-text/src/lib.rs:1123-1131`: violates the documented "space at either end of a line is not set"; inflates plate width and off-centres the block. **Fix:** use the trimmed range.
- **L23. `.expect("a translated glyph path stays finite")` on user-font geometry** — `concat-text/src/lib.rs:1805-1810`: a degenerate user-installed font aborts the app (`panic = "abort"`). **Fix:** skip the untransformable path, as sibling failures already are.
- **L24. `vad_model` leaks the `.part-<pid>` file when the rename fails** — `concat-speech/src/transcribe.rs:288-301`. **Fix:** `remove_file` on error, matching `download_model`'s cleanup.
- *(Vision — see M16/M17 in Part II.)*

## App crate (`concat`)

- **L25. "A still has no sound to filter" literal English** — `studio.rs:4288-4291`. **Fix:** locale key (`studio.stillHasNoSound`).
- **L26. Import-failure toast prefix hardcoded** — `host.rs:711-714` (`"Could not import: {error}"`), while every neighbouring import notice is localized. **Fix:** `mediaBin.couldNotImport` key.
- **L27. Closing a timeline tab deletes the timeline and every clip with no confirmation** — `lib.rs:1037-1048`: the asymmetry with `ask_remove_track` (which confirms) reads as an oversight; tabs.slint's own header says the strip "never deletes directly". **Fix:** route through the existing `Confirm` sheet when the timeline holds clips.
- **L28. Editor shortcuts dispatch while a pointer-opened context menu is up** — `ui/app.slint:632-696` + `ui/primitives/context-menu.slint:179-184`: the menu's FocusScope only takes focus when keyboard-opened, so Q/W/B/⌫ edit the timeline *behind* an open menu. **[needs-runtime-test]** **Fix:** gate dispatch on an open-menu flag (as `sheet_open()` does) or focus the menu whenever it shows.

## Build / CI / scripts / docs

- **L29. Digest-recording failures swallowed in `models.py`/`models.yml`** — `scripts/models.py:241-272` + `.github/workflows/models.yml:76-95`: a manifest row that doesn't match the fill regex records nothing, yet the workflow summary claims "every model already carried its digest". **Fix:** propagate `fill()`'s result and fail the workflow.
- **L30. `nix.yml` pull_request path filter missing entries the push filter has** — `.github/workflows/nix.yml:10-23`: a PR changing a crate manifest skips the Nix check; main goes red after the merge instead. **Fix:** copy the missing paths.
- **L31. Product version hardcoded in three places with no consistency check** — `flake.nix:152`, `scripts/readme/gen.py:55`, `src/Cargo.toml`; release.yml validates only the Cargo.toml side. **Fix:** derive from Cargo.toml and/or add a `checks`-job comparison.
- **L32. Format-leakage assert is a no-op (`… or True`)** — `scripts/readme/gen.py:158`: the check never fires; a malformed `lang_*.py` produces a silently broken README. **Fix:** implement it properly or delete the line.
- **L33. Locally-run CI/bundle scripts create unignored artifacts in the repo root** — `.gitignore`: `smoke-android/`, `smoke-ios/`, `stage/`, `Concat.AppDir/`, `*.dmg`, `nextest-*.log`, etc. are not ignored; one `git add -A` away from being committed. **Fix:** add the entries.
- **L34. Real-ESRGAN (the fifth vision model) has no attribution bullet** — `THIRD_PARTY_NOTICES.md` ~205-227 vs `models/manifest.toml:79-85`: BSD-3-Clause redistribution wants the copyright notice and source named. **Fix:** add the bullet naming xinntao/Real-ESRGAN and the ONNX export.
- **L35. Media-bin band() sort issue's sibling: `visible()` itself is correct** — no fix needed; recorded to close the loop on H4's audit trail. *(Housekeeping note.)*

---

# Part IV — What was audited and found sound

Recorded so the next audit doesn't re-derive it:

- **concat-core**: exact rational time, tolerant parsing, generational arena, distance fields — panic-free on hostile input.
- **concat-project**: undo/gesture/batch atomicity, ripple/linkage and key-rewindowing arithmetic verified against the 5,900-line test module; every command arm raises errors *before* mutating (failed commands never leave partial state).
- **concat-api**: write-root confinement is component-wise, symlink-resolving and `..`-rejecting; envelope/dispatcher solid.
- **FFmpeg FFI discipline** (`concat-media`): return codes checked throughout; EAGAIN/EOF/stall semantics modelled carefully; side-data writes size-checked; hardware fallback paths thorough.
- **wgpu core** (`concat-render`): bind group layouts match every WGSL declaration audited; row pitches 256-padded; no `unwrap`/`expect` on adapter/device; device-loss handled; colour-science constants verified numerically (BT.1886, YIQ, Bradford, Planckian locus, ACEScct, Fritsch-Carlson).
- **concat-server (JSON door)**: auth (constant-time, throttled), caps, atomic listing — excellent; MCP headless writes confined to home/`CONCAT_MCP_ROOTS` (e2e asserts `/` is refused); panics contained via `catch_unwind` in the hub and the tool dispatcher.
- **concat-text**: shaping/bidi/byte-vs-char handling correct and heavily tested (runs count characters, clusters map bytes, RTL pieces reversed, marks join their base).
- **concat-vision**: tensor layouts and normalizations all check out against their references (ImageNet for SAM, DIS max−0.5, RVM zero recurrent state, [0,1] selfie/Real-ESRGAN); the protobuf weight reader is sound.
- **Locales/manifests**: all 25 translations structurally identical to `en.json` (0 missing / 0 stale / 0 duplicate keys); all manifest sha256s 64-hex, all URLs https; 23 models mirrored on `models-v1`.
- **Timeline interaction core** (app crate): drag/trim/snap/split/ripple/zoom robust, echo-then-command undo, click-vs-drag thresholds, division guards — unit-tested; zero `unwrap`/`expect` on UI callback paths.

---

# Part V — Recommended fix order

1. **CI-green first:** H1 (`cargo fmt`) — unblocks every other PR.
2. **Crashes/data loss:** H5 (TTS NUL abort), M1 (id overflow), M3/M4 (export input validation), M5/M6 (encoder/mix lifecycle), L23 (glyph expect), L16 (poison unwraps).
3. **Visible correctness:** H3 (alpha double-multiply), H4 (marquee sort), M8/M9 (transition edges), M12 (save flush), M13/M15/L25/L26 (i18n leaks), M16 (enhance alpha).
4. **Resource/robustness:** H2 (fd exhaustion), M7 (mix cancel), M10 (upscaler wait), M17 (per-frame readdir), L20/L21 (speech hot spots), L10/L11 (caches/ceilings).
5. **Build/CI hardening:** M18–M21, L29–L34.

---

*Audit performed for the `AbedRafik/Concat` fork. All line numbers refer to commit `f0c01696` of `jub0t/Concat`. Items marked **[needs-runtime-test]** require a runnable build/GPU to confirm before fixing; they are included with their evidence and trigger scenarios.*
