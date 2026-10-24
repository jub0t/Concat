## Engine: media, render, export, core, effects, host playback/preview/posters. main 2606aff1, 2026-10-24

Labels: **Verified** means I read it at the line. **Reasoned** means I followed the code path; nothing was run. **Suspected** means the reason is given. Nothing was built or run: this is a read-only audit, and another agent held the target dir. wgpu-core and wgpu-hal 30.0.1 were read from `~/.cargo/registry`.

Since c41865e these crates have 29 non-merge commits. That is about +5 700 / −1 000 lines.
- `concat-render/src/gpu.rs` and `concat-effects` changed only in their copyright header.
- The work is in `concat-media` (decode, pool, audio, samples, encode, feed, ffi, probe, prefetch, svg), `concat-export/src/lib.rs` (+1 054) and `concat-host` (playback +434, media +275).

### 1. The 20 October findings

| Finding | Status | Evidence now |
|---|---|---|
| H7 export recovery opens the second device that crashes hybrid laptops | **Unchanged** | `recovered(1)` is still `WgpuCompositor::new()` (`concat-export/src/lib.rs:1612-1624`). It is called from the lost-device loop (`lib.rs:1946-1965`). The sibling still shares the window's error counter (`concat-render/src/gpu.rs:960-963`). |
| M1 MediaCodec seats first-come, never reclaimed | **Partly, and one part worse** | Better: the export closes idle preview readers before it starts (`concat/src/panes/export.rs:480-490`, 0c86f62), and bin cards decode in software (`concat-host/src/media.rs:925-951`). Unchanged: 3 seats, no priority, no reclaim (`concat-media/src/hardware.rs:268, 320`). Filmstrip `read_through` and `still_at` still ask for hardware (`media.rs:503, 888`). Worse: #505 allows 3 readers per file (`pool.rs:64`), so one file can now hold every seat. |
| M2 / #448 adjustment layer turns transitions into dissolves | **Fixed** (d8b024f4) | `composite_treated` hands live treatments to `combine_transition`, which grades below and above the incoming lane (`lib.rs:2139-2149, 2219-2238`). The monitor's GPU fast path steps aside when a treatment is live (`lib.rs:2347-2353`). There is a GPU test (`lib.rs:2676+`). Still left: only the first live span gets its shader (`lib.rs:2145`). |
| M3 GLES export dies after two 2.15 s waits | **Unchanged** | `DEVICE_WAIT` is 30 s (`gpu.rs:296`), passed to `poll` (`gpu.rs:1433-1440`). wgpu-hal is still 30.0.1, so the clamp still applies. |
| M4 default export: single-pass ABR, `veryfast` | **Unchanged** | `concat/src/panes/export.rs:454-457`. The x264 dictionary is `b`/`maxrate 1.5×`/`bufsize 2×` with no CRF (`encode.rs:831-835`). |
| N2 chained transitions climb into the next track's lane | **Fixed** (#527) | Lanes are now spread by the longest chain + 1, with cuts resolved in time order (`lib.rs:682-695, 873-879`). Tests cover four transition pairs and a logo over a chain (`lib.rs:2957+, 4193+`). |
| 12Oct#4 / H1 WARP+FXC compiles unbounded, Cancel deaf | **Partly** | The trigger is narrower: safe start now lasts one build (f26ebf64, window crate). The engine is unchanged: FXC pin at `gpu.rs:861-862`, a synchronous `shader()` at `gpu.rs:2615+`, Cancel checked only per frame (`lib.rs:1801`), no pipeline cache. |
| Device-lost panic (#475) | **Unchanged in the engine** | No engine commit addresses it. The compositor's handlers log instead of panicking (`gpu.rs:916-934`). 3f6db386 stopped sharing a WARP device with Skia (window crate). That is the one path the repo shows wgpu panicking in its queue. A hardware device shared with Skia is unchanged. #475 is open with no log. |
| M5 export reader pool fixed at 512 MB on any device | **Unchanged, slightly worse** | `ReaderPool::with_defaults()` (`lib.rs:1754`, `pool.rs:668-671`). Each warm reader now also pins `last` and `ahead` frames outside the byte budget (`pool.rs:374-377`). |

**Tally (9):** 2 fixed, 2 partly, 5 unchanged.

### 2. New findings

#### HIGH

**E-H1. On Android, a safe start leaves the phone with no compositor. The monitor draws nothing and every export fails until the next build.** *Verified path. The phone behaviour is reasoned.*
- 7df9f8f2 turns safe start on for Android (`concat/src/lib.rs:2643-2645`). Its comment says that on Android `CONCAT_GPU=software` "means OpenGL ES in place of Vulkan (the Skia patch reads it), not the CPU" (`lib.rs:2657-2662`).
- `concat-render` reads the same variable as "software adapters only" (`gpu.rs:828-833`).
- wgpu-core keeps only `DeviceType::Cpu` adapters under `force_fallback_adapter` (`wgpu-core-30.0.1/src/instance.rs:511-518`). A GLES adapter counts as `Cpu` only for SwiftShader, llvmpipe or Mesa offscreen (`wgpu-hal-30.0.1/src/gles/adapter.rs:144-155`). A real phone has none of those.
- Android lends no window device to the engine (`concat/src/platform.rs:671-684` returns `None`). So the export goes to `best_compositor()` and fails with "this device's GPU could not be used to draw with" (`concat-export/src/lib.rs:1573-1592`).
- The monitor's frames go through `headless()`, which logs "previews have nothing to draw on" (`lib.rs:1643-1660`). Effect cards are skipped (`concat-host/src/cards.rs:398-415`).
- **Scenario:** the OS kills the app in its first 15 s (low memory, or a swipe from recents), or #507's Vulkan crash happens once. On every launch of that build the window opens on GLES as intended, but the preview is blank and no export can run. This is on the platform with the most downloads.
- **Fix:** give the window and the compositor separate keys (for example, `CONCAT_WINDOW_GLES`). Or, on Android, have `own_api_first` read the variable as "GL backend first", not "software only".

**E-H2. H7 carried: export recovery opens a fresh HighPerformance device.** Unchanged; see the table.

#### MEDIUM

**E-M1. #462 is fixed only for footage at the timeline's rate. Fast sources still open two or three readers on plain playback.** *Reasoned.*
- `claim_cost` forgives a reader up to `prefetch::AHEAD` = 8 frames past the target (`pool.rs:335-343`, `prefetch.rs:38`). That 8 is *output* frames of lookahead, but the pool counts in *source* frames at the file's cadence.
- The race spans the workers' neighbouring *output* frames: 2-4 workers (`prefetch.rs:261`). The table shows where it falls through.

| Source | Source frames per output frame | Output frames the slack covers |
|---|---|---|
| 30 fps on a 30 fps timeline | 1 | 8 |
| 60 fps | 2 | 4 |
| 120 fps | 4 | 2 |
| 240 fps slow-motion | 8 | 1 |
| 4× speed ramp | 4 | 2 |

- bac31a3 makes this more common. `pick_cadence` takes `r_frame_rate` whenever it is above the average (`probe.rs:285-289`), so 1080i MTS/M2TS footage ("29.97 fps, 59.94 tbr") is indexed at the field rate.
- **Scenario:** a 120 fps phone clip, or a 2-4× ramp, on a 4-worker desktop. Playback opens extra readers, and they decode the same stretch twice.
- **Fix:** scale the slack by source frames per output frame, or by `workers × step`.

**E-M2. Audio seeks are not checked for landing. With 93c18b72, a late landing becomes silence at the head of the clip.** *Reasoned. The late landing itself is documented by d02b7565's own comment (`decode.rs:1070-1082`).*
- Video now retreats when a TS/PS seek lands a GOP late or at EOF (`decode.rs:1083+`). The sound side seeks once and trusts it, in both readers: `AudioDecoder` (`samples.rs:209-217`) and the export mix (`audio.rs:759`).
- Before, a late landing slid the sound early. Now `late` and a negative `lead` pad the overshoot with silence (`samples.rs:94, 247-252`; `audio.rs:383, 785`).
- A seek that lands at EOF gives a silent clip.
- No test seeks into a file with both streams: the TS audio fixture is audio-only, and the TS video fixture is video-only.
- **Scenario:** a camera MTS clip trimmed from the front. In the export and in the preview, its sound starts up to a GOP late, or never.
- **Fix:** the same landing check and retreat on the audio side. Or seek on the audio stream's own index rather than −1, which TS resolves to video.

**E-M3. The phone's software rerun (0c86f62) may not be able to take the exports most likely to stall.** *Suspected: AOSP limits from memory; needs a device check.*
- The rerun goes straight to `c2.android.avc/hevc/av1.encoder` (`encode.rs:621-634, 653-662`).
- As far as I recall, AOSP's C2SoftAvcEnc caps the frame at about 2560 a side and C2SoftHevcEnc at 1080p, both 8-bit. `c2.android.av1.encoder` exists only on newer Android.
- A 4K or 10-bit HEVC export whose chip stalls renders every frame twice, then fails with "the slower software encoder … failed too".
- The 'pass loop itself has no end-to-end test. Only `again_in_software` is tested (`lib.rs:3665`).

**E-M4. M1 carried, now with more seat demand per file.** See the table. On Android, a transition within one file, plus a scrub, can take all 3 seats, so the next clip under the playhead decodes on the CPU.

**E-M5. Carried unchanged.** M3 (GLES waits), M4 (default bitrate) and M5 (export pool).

#### LOW

- **L1. A bus placed for a window that starts inside it runs past its end.** `bus_stage` clamps `start` to the window and then adds the full duration (`audio.rs:321-322`). Playback windows start at the earliest layer of a group, so this cannot happen today. Latent for any future `mix_pcm` caller. Also, each bus chain runs over the whole timeline, gated to silence outside its span: an `asplit=3` per bus, so the cost grows with timeline length, not span length.
- **L2. Posters can fill the worker kept for playback.** `Poster` is not background work (`prefetch.rs:83-87`), so a big import's posters can take every worker. A poster on a slow volume (a network share, an iCloud placeholder, a 4K HEVC probe on a phone) then makes playback wait for one to finish. The measured gains (7b2d30e) justify the priority; a cap of `workers - 1` would keep both.
- **L3. Damaged-packet skipping never trips on a stream that alternates.** A good send resets `damaged` (`feed.rs:108-117`). A stream whose every packet sends fine and decodes to `InvalidData` is read to EOF under the reader's lock, where it used to fail at once. The cost is time, not a hang.
- **L4. The racing claim re-decodes frames.** A reader claimed from behind keeps its furthest `aim` (`pool.rs:561-565`) after `pull` has sought it back. The next claim then rolls up to 8 frames it already decoded. The cost is CPU only.
- **L5. SVG drawings are cached in a shared, predictable temp path** (`svg.rs` `drawing_of`: `temp_dir()/concat-svg/<hash>.png`). On multi-user Linux, another user can plant the PNG. `load_system_fonts` runs on every first draw.
- **L6. Projects imported before b8337561 may keep their squeezed sizes.** They store the 1440-wide media size, and export `decode_sizes` come from the clip's stored dimensions. Unverified whether the app probes again on open.

### 3. Commits reviewed

| Commit | Verdict |
|---|---|
| #531 fe2be741 `timeline_extent` | Correct. It uses the same `quantise_at` + `quantise` as `build_timeline` (`resolve.rs:291-292`, `lib.rs:1281-1287`), and the mix takes the same end. Tested. |
| #531 167a7331 borrowed plan | Correct. It clones only when a treatment is live (`lib.rs:2150-2154`), and the retry after a lost device redraws from the same borrow. |
| #527 `lane_spacing` | Correct. A chain of n cuts climbs n lanes with n+1 spacing (`lib.rs:873-879`). The sound-layer ordering by track survives the multiply. Tests cover four transition pairs. |
| #505 + 2b6eb23 readers per key | Sound locking: claim under the short lock, decode under the reader's own; no nested locks, so no deadlock. The LRU is a `Vec`, which is fine at 8. The units gap is E-M1; the seat cost is E-M4. |
| 0c86f62 software rerun | The loop is bounded (`software_only` gates a second rerun, `lib.rs:2015`). Decoders are dropped each pass, the encoder is dropped before the reopen, and progress resets in the window. Open: E-M3. |
| d02b7565 TS/PS retreat ladder | Bounded: steps of 1, 4, 16 … s, then `seek_top` clears `landing` (`decode.rs:1083-1115`). Codec flush on retreat is correct after EOF. Byte seek only for `AVFMT_TS_DISCONT`. Good. |
| b8337561 SAR | Matches `av_guess_sample_aspect_ratio` precedence, with a sane band (`ffi.rs` `sample_aspect`). The stretch comes before rotation. Tested with an HDV fixture. See L6. |
| 39ebb75e ms-timestamp tolerance | Capped at a quarter of an output frame (`decode.rs:1427`), so the next frame is never pulled in early. Tested with MKV. |
| 1dac19b7 held frame before target | Holds at most one undecoded hardware frame (`short` or `stashed`, `decode.rs:1236-1249`), well inside D3D11VA's spare surfaces, so no surface exhaustion. Frames handed on are downloaded first. |
| bac31a3 VFR cadence | Correct for screen recordings. Over-eager for interlaced and jittery files (E-M1), but `held_at` keeps the picture right. |
| 7c113230 damaged packets | Right direction, bounded at 64 in a row. See L3. |
| 93c18b72 one origin per file | Consistent across picture, preview sound, waveform, transcript and export mix (`ffi::origin` in `decode.rs`, `samples.rs:200`, `audio.rs:751`). The audio seek gap is E-M2. |
| Poster/Waveform priorities | Sound ordering, with tests. See L2. |
| ccc6131c `card_still` | Good: software, one keyframe, with exact-frame fallbacks for black, unreadable or later frames. Tested against a scene-cut stream. |
| 7dc91a92 filmstrip keyframes | Good: it keeps the result only if every tile came from its own slice, and otherwise falls back to the exact pass. It decodes on the platform's video hardware (`keyframes_only` keeps it off MediaCodec). |
| 2606aff1 sound layers | Graph is correct. The dry path minus the ramped dry plus the ramped wet is exact outside the span (a test checks it sample for sample). The playback span is the export's own `run_mix`. Clips go silent only once the span is cached (`playback.rs:805-809`). Cache is budgeted. See L1. |
| e353fe57 WAV channel layout | Correct. It is applied before the buffer source is described, in both readers. |

### 4. Numbers

| Crate | `#[test]` now (20 Oct) | Ignored |
|---|---|---|
| media | 143 (113) | 1 |
| render | 101 (101) | 0 |
| export | 46 (38) | 0 |
| core | 64 (60) | 0 |
| effects | 72 (72) | 0 |

Big files: export `lib.rs` 4 358 (+588 since 20 Oct), `gpu.rs` 3 695, `encode.rs` 2 813, `decode.rs` 2 488, `pool.rs` 1 872, `audio.rs` 1 701, `playback.rs` 1 779.

### 5. Scores

| Aspect | 20 Oct | Now | Why |
|---|---|---|---|
| Code quality | 7 | 7 | The #457 fixes are careful and bounded (retreat ladder, damage counter, slack capped at a quarter frame). But `CONCAT_GPU` means two different things in two crates (E-H1), and `claim_cost` mixes units (E-M1). |
| Design architecture | 7 | 6.5 ↓ | Readers routed by aim and a sound-layer bus over the sum are good designs. Seats still have no priority, recovery still opens a second device, and safe start has no single owner. |
| Performance | 6 | 7 ↑ | Measured, large wins: card pictures 8-15 s → 0.1-0.2 s, phone filmstrips 10-13 s → 0.15 s, and no plan clone per frame. Offset by extra readers for fast sources and a full-timeline cost per bus. |
| Scalability | 5 | 5 | Fixed 512 MB export pool; 3 readers per file × 8; frames pinned outside the budget; the audio mix still holds the timeline (N4). |
| Testing | 6.5 | 7 ↑ | A real fixture for each #457 format (TS, MKV, HDV, VFR, late picture and late sound), a chained-transition test and a sound-bus sample test. Missing: Android safe start with the compositor, audio seek in a muxed TS, fast-source claims, the 'pass loop end-to-end. The device test is still `#[ignore]`. |

### 6. Must fix before a release

1. **E-H1.** One launch that dies early leaves an Android build with no preview and no export. It is a one-line separation of meaning, on the priority platform.
2. **E-M2 needs a check.** Export a camera MTS clip trimmed from the front, and listen to the first second.
3. **E-M3 needs a device check.** A 4K export through `c2.android.avc.encoder`.

### 7. Not checked

- MediaCodec and C2 software encoder behaviour on real phones; I did not test their size limits.
- Whether the phone shows the Settings GPU switch that would clear a safe start.
- #475's real cause: there is no log in the issue.
- Whether old projects probe HDV clips again (L6).
