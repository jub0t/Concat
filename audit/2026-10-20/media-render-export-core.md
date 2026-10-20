## media, render, export, core (+ vision frames): HEAD ce64dae, 2026-10-20

Labels: **Verified** = I read it at the line. **Reasoned** = I followed the code path; nothing was run. **Suspected** = the reason is given. Nothing was built or run (read-only audit). wgpu-hal / wgpu-types 30.0.1 sources were read from `~/.cargo/registry`.

Since 2026-10-15 these crates have 26 commits: +6 296 / −252 lines over 32 files (`git diff --stat 11e1bd3 HEAD`). The ones that matter:
- 2969e02: Android decodes on MediaCodec.
- ac8a654: feed.rs.
- b18c2fe and a9b2df6: the FXC pin.
- d3c2dcd: GLES uniform alignment.
- b5e6c08: the safe-start hook in render.
- 89258aa: average bitrate.
- 4f69340 / 95d6ea0: the VFR rate.
- a72e8ec: `Rational::parse`.
- 16a3d13: masks.
- 03dac1f: backgrounds.
- 72cbfe5 / 4edd466: stabilization.
- 0f63fca: adjustment-layer test.
- 4451548: #448 test.
- ce64dae: transition knobs.

### 1. Previous findings re-verified

| ID | Status | Evidence at HEAD |
|---|---|---|
| 12Oct#4 FXC compile unbounded, repeated, Cancel deaf | **UNCHANGED, exposure WORSE** | `shader()` compiles synchronously with no deadline (`concat-render/src/gpu.rs:2615-2690`, `block_on(scope.pop())` :2681). Cancel is checked only at the top of a frame (`concat-export/src/lib.rs:1687`). Every compositor starts with an empty map (`gpu.rs:1387`), and `sibling()` makes a fresh one (:960-963), so every export recompiles. b18c2fe/a9b2df6 now force FXC on every Windows machine (`gpu.rs:861-862`, `concat/src/gpu.rs:69-70`). b5e6c08 sends crashed launches to WARP+FXC for good (see H1). |
| 12Oct#7 GLES 30 s wait is really ~2.15 s | **UNCHANGED, exposure WORSE** | wgpu-hal 30 clamps it: `timeout_ns.min(i32::MAX)` (`wgpu-hal-30.0.1/src/gles/fence.rs:162`, `device.rs:1757-1763`). `DEVICE_WAIT` still documents 30 s (`gpu.rs:288-296`). d3c2dcd makes effects and transitions actually run on GLES, so slow GLES frames now happen in practice (M3). |
| 12Oct#9 HEVC/AV1 export opens NVENC first | **UNCHANGED** | `concat-media/src/encode.rs:128-139`. The export still passes `hardware: true` (`concat-export/src/lib.rs:1648`). |
| 12Oct#10 a validation error counts as a lost GPU, then second device, then WARP | **UNCHANGED** | `gpu.rs:3543-3573`; `concat-export/src/lib.rs:1521-1550`. |
| 12Oct#11 `combine` unscoped, refused passes skipped silently, lent counter never moves | **UNCHANGED** | `combine` has no error scope (`gpu.rs:3584-3653`). Refused passes: `gpu.rs:2686, 2809`. `with_device` counter: `gpu.rs:1413`. The window device's handler only logs (`concat/src/gpu.rs:133`), so on Windows (the lent sibling) an uncaught error never fails an export frame. |
| 12Oct#31 8-bit phone fallback clips HDR | **UNCHANGED** | Warn only: `gpu.rs:903-911`. |
| 12Oct#33 DEVICE_WAIT doc and two read_back docs | **UNCHANGED** | `gpu.rs:288-296`. |
| 12Oct#5 a preview layer that fails on the export side is dropped silently | **UNCHANGED** | Failures are collected but only reported when *every* layer fails (`concat-export/src/lib.rs:2296-2309`). |
| 12Oct§7 encoder EAGAIN retry | **FIXED (as before)** | Still no phone export in CI. |
| 11Oct#8 uncaptured GPU errors | **PARTLY (as before)** | Remainder is 12Oct#11. |
| 11Oct#11 mux scratch names collide | **UNCHANGED** | `concat-export/src/lib.rs:1180-1182`: two exports of the same stem in the same folder clobber each other. |
| 11Oct#12 paced and pooled decode disagree past clip end | **UNCHANGED** | Uses the output frame's duration (`decode.rs:1183-1190`). |
| 11Oct#13 `pick_rate` VFR | **PARTLY FIXED** | 4f69340/95d6ea0 round an oversized average instead of falling to the 90 kHz grid (`probe.rs:241-245`, `concat-core/src/time.rs:361-386`). Still no VFR media fixture, only arithmetic tests. A VFR clip now reports e.g. 61.219 fps (unverified whether the timeline ever adopts it). |
| 11Oct#14 `MAX_SIDE` doc | **UNCHANGED** | `concat-core/src/frame.rs:27-31` still says every GPU meets 8192. GLES phones are 4096 (`gpu.rs:970-972`). |
| 11Oct#21 CPU fallback docs | **UNCHANGED** | `concat-export/src/lib.rs:19` ("CPU compositor as the always-correct fallback") against `gpu.rs:808` ("since the CPU compositor went"), while :782 and :1418 still promise CPU answers. `render()` returns black (`gpu.rs:3532`). |
| 11Oct#22 dead core code | **UNCHANGED** | `concat-core/src/timeline.rs:329` (`source_duration` is used only by tests) and :510-517 (`Project`, unused). |
| core `Neg`/`ceil` on `i64::MIN` | **UNCHANGED** | `time.rs:158-160, 251-258`. (`parse` itself was fixed in a72e8ec, verified.) |
| N1 Android never decodes on MediaCodec | **PARTLY FIXED** | 2969e02 opens `h264/hevc/vp9/av1_mediacodec` by name for 8-bit 4:2:0 (`hardware.rs:239-250`, `decode.rs:684-731`). The log now names the decoder only after a real open (`hardware.rs:427-434`). **Still CPU:** 10-bit HEVC (`hardware.rs:241`), which is the HDR phone footage that most needed the chip; keyframe-only readers; and the 4th reader onward. The real-file device test is `#[ignore]` (`decode.rs:1850`) and appears in no script or workflow (`grep` of `scripts/` and `.github/`: zero hits). New problems in this code: M1, L1. |
| N2 chained transitions climb into the next track's lane | **UNCHANGED, now worse in effect** | `concat-export/src/lib.rs:889` (`b.track = a_track + 1` with `a.track` already bumped). Still no test. A packaged span's `to_track` (lib.rs:808) then lands on track t+1's lane, and `transition_stages` forces every layer on that lane to opacity 1 (`lib.rs:2008-2014`). A caption on track t+1 is pulled into the transition at full opacity. |
| N3 HDR proxy opens NVENC in the background | **UNCHANGED** | `concat-host/src/proxy.rs:238` (`hardware: hdr`). The proxy decode is now software (:218), which is good. |
| N4/M7 audio mix holds the timeline's sound, Cancel deaf | **UNCHANGED** | `concat-export/src/lib.rs:1202-1210`, `audio.rs:371+`. |
| N5/M8 crossfade heard only in the file | **UNCHANGED** | `resolve_transitions` is still private (`lib.rs:621`). Playback uses `flattened_clips()` (`concat-host/src/session.rs:306`). |
| N6/M12 `encodable()` opens MediaCodec encoders on the UI thread | **UNCHANGED** | `encode.rs:162-180`; the doc admits "before the window shows". |
| N7/M9 FFmpeg ERROR/WARNING dropped | **UNCHANGED, impact WORSE** | `ffi.rs:101-105`. It now also hides the MediaCodec *decoder's* configure and dequeue reasons. Only "would not open … Invalid argument"-style summaries reach the log. |
| N8 `EffectClock` doesn't forward `lost`/`deliver_hdr` | **UNCHANGED** (latent) | `lib.rs:1905-1940`. |
| N9 decoders drop a packet on EAGAIN | **FIXED** (ac8a654) | `feed.rs:76-107` keeps and resends. Every decode loop goes through it: no `send_packet` call is left outside feed.rs. Fake-codec tests are in `feed.rs:176+`. |
| N10 ~45/80 MB allocated per 4K frame | **UNCHANGED** | `gpu.rs:3492-3510`, `encode.rs:1552-1559`. |
| N11 prefetch "single-threaded" doc | **UNCHANGED** | `prefetch.rs:241` against `decode.rs:826-837`. |
| N12 Android SDR proxy holds a MediaCodec encoder | **UNCHANGED** | Android's list is MediaCodec-only, whatever `hardware` says (`encode.rs:106-112`). |
| N13 volume doc drift | **UNCHANGED** | `animate.rs:334` `volume_at` still has no caller. |
| M14 second Vulkan device per export or card round | **UNCHANGED** | `concat-host/src/preview.rs:146-153`. |

**Tally (28):** 1 fixed (N9), 1 still fixed (§7), 3 partly (N1, 11Oct#13, 11Oct#8), 19 unchanged, 4 unchanged with worse exposure (12Oct#4, 12Oct#7, N2, N7).

### 2. New findings

#### HIGH

**H1. On a Windows machine whose last launch died while starting, every export runs on WARP+FXC permanently. Effect compiles then take minutes with Cancel deaf.** *Verified path; the timing comes from the repo's own note.*
- How it happens:
  - b5e6c08's safe start (`concat/src/lib.rs:2459-2474`) sets `CONCAT_GPU=software` for good whenever the previous process died within 15 s, for *any* reason: a driver, a panic=abort in project load, Task Manager, a power cut.
  - `own_api_first` honours it for the compositor too (`concat-render/src/gpu.rs:831-833`).
  - WARP compiles through FXC (b18c2fe pin, `gpu.rs:861-862`).
- The repo's own test note says that on WARP each package compile "ran past five minutes … every compile going through FXC … and never finished" (`gpu/tests.rs:35-40`). That is why those tests skip WARP (`heavy_gpu`, `tests.rs:41-48`).
- An export compiles every effect again (12Oct#4), synchronously, before the frame's cancel check (`concat-export/src/lib.rs:1687`).
- **Scenario:** a user whose GPU driver once crashed at start exports a clip with a Gaussian blur, glow or blurred background (03dac1f puts `concat.gaussian-blur` behind every "Blur" background, `lib.rs:1465-1470`). The progress bar sits at 0 for minutes and Cancel does nothing.
- The only way back is a Settings switch the user has no reason to find.
- **Fix:** keep the compositor on the GPU when only the window crashed, or at least probe the GPU again. Compile off-thread with a deadline. Cache pipelines across exports (`PipelineCache`, or keep the sibling's shader map).

**H2. On Windows, the export draws on a sibling of the window's device. Any GPU error in the window marks the export's frame failed, and recovery opens the second device that #202 says kills NVIDIA laptops.** *Partly verified, partly reasoned.*
- `sibling()` shares the `uncaught` counter (`gpu.rs:960-963`).
- `render()` counts any increment during the frame as a failed frame and kills the compositor (`gpu.rs:3543-3573`).
- For the lent (window) device the counter never moves (12Oct#11), so this is latent on that exact path.
- Separately, any readback timeout or OOM on the lent device sends the export to `recovered(1)` = `WgpuCompositor::new()` (`concat-export/src/lib.rs:1530-1531`): a fresh HighPerformance DX12 device. On hybrid laptops that is the dGPU while the window is on the iGPU, which is exactly the "second road through the driver" that `render_on`'s own doc (`lib.rs:1072-1078`) says ended in a crash (#202).
- **Scenario:** an 8 GB card runs out of memory mid-export (#223/#297 class). Instead of retrying on the window's device, the export opens NVIDIA directly and the app dies with nothing in the log.
- **Fix:** recover first on `Monitor::off_thread()` again (a new sibling of the window's device), then WARP. Never `new()` when the export was lent a device.

#### MEDIUM

**M1. MediaCodec seats are first-come and never reclaimed. Idle warm readers and background jobs hold all three, so the clip under the playhead decodes on the CPU for the whole life of its reader.** *Verified path; impact reasoned.*
- 3 seats process-wide (`hardware.rs:268`). Seats are taken at open (`decode.rs:690-697`) and released only when the reader drops or falls back.
- Warm readers are closed only after 10-15 s idle (`concat/src/lib.rs:2388-2403`, `pool.rs:599-622`). A software reader never moves onto the chip when a seat frees.
- `DecodeOptions::default()` is `HwPolicy::Preference` (`decode.rs:152-158`), so other jobs take seats too:
  - stabilization analysis (`concat-host/src/stabilize.rs:174`);
  - filmstrip `read_through` (`concat-host/src/media.rs:490`);
  - `still_at` one-frame grabs (`media.rs:824`), the "chip's worst case" the commit excluded only for `keyframes_only`;
  - cutout and brush jobs (`cutout.rs:279`, `brush.rs:206`);
  - the export's own decoders and its private pool (`concat-export/src/lib.rs:1684, 1774`).
- **Scenario:** a phone edit of short clips (the TikTok shape). After three cuts, every newly opened reader is software, so playback stutters exactly where the commit claims a win. During export, preview readers or a stabilization pass hold the seats, so the export decodes 4K on the CPU.
- **Fix:** give seats by priority (playhead and export over background), make background and one-frame readers `in_software()`, and let a reader take a freed seat at its next seek.

**M2. Any live adjustment layer turns every packaged transition into a dissolve, so today's knobs (ce64dae) silently do nothing under a timeline-wide grade.** *Verified.*
- `composite_treated` skips the shader when *any* treatment covers the instant (`concat-export/src/lib.rs:1958-1964`). The monitor does the same (`lib.rs:2174-2180`).
- The comment calls it "a rare pairing". A grade adjustment layer over the whole edit is the most common way adjustment layers are used (0f63fca just tested that path).
- `transitions.iter().find(..)` (`lib.rs:1960`, :2181) also runs only the *first* live span, so two transitions overlapping in time on different tracks give one shader and one dissolve.
- Only `transition_pass` and `combine` are tested with knobs (`gpu/tests.rs:838-864`, `lib.rs:3346-3354`). No test checks that knobs survive `composite_treated` or the monitor.
- This is the code-level candidate for #448 ("all transitions look the same, just a fade"), which 4451548 could not reproduce because its test bypasses this gate. Other candidates are a refused pipeline, which falls back silently (`gpu.rs:2804-2810`), and a GLES device before d3c2dcd.

**M3. A GLES phone's export dies when one frame takes over 2.15 s. That is now reachable because d3c2dcd lets effects run on GLES.** *Verified wgpu clamp; scenario reasoned.*
- The 30 s `DEVICE_WAIT` becomes `i32::MAX` ns (`wgpu-hal gles/fence.rs:162`).
- The first "loss" opens a new device. The second asks for `software()`, which an Android phone does not have, so the export fails: "the GPU device was lost part way through the export, and this device's GPU could not be used…" (`concat-export/src/lib.rs:1530-1537, 1496-1497`).
- **Scenario:** a Mali/Adreno GLES fallback phone exports 1080p with a blurred background plus an effect. Two frames over 2.15 s and the export is gone, on the platform with the most downloads.
- **Fix:** on GLES, poll in a loop up to `DEVICE_WAIT`, not one clamped wait.

**M4. Export bitrate moved from CRF to single-pass ABR at YouTube's *upload* rates, with x264 `veryfast`. That is a visible quality drop for the default export.** *Verified settings; the quality impact is the well-known behaviour of x264 1-pass ABR.*
- The default is now `RateMode::Average` with `preset "veryfast"` (`concat/src/panes/export.rs:441-452`). 1080p30 H.264 gets 8 Mb/s (`export.rs:336-353`).
- x264 runs `b/maxrate 1.5×/bufsize 2×` with no CRF (`encode.rs` `libx264 … if average`, 89258aa).
- Previously it was CRF 16, which the sheet's own estimate put at about 16 Mb/s at 1080p30 (`encode.rs` `implied_kbps` doc).
- Single-pass ABR is x264's least efficient mode: it starves the first seconds and busy scenes. These are master files that platforms re-encode, so the generation loss compounds.
- Defensible as a product choice for predictable size, but it should be CRF-capped VBV (`crf` + `maxrate`) or 2-pass, not bare ABR.

**M5. The export's private reader pool is a fixed 512 MB regardless of device memory.** *Verified.*
- `ReaderPool::with_defaults()` (`concat-export/src/lib.rs:1684`) gives 512 MB and 8 readers. It does not use `sized_for` (`pool.rs:589-591`). It sits on top of the app's pool (256 MB to 1 GB).
- It fills with any reversed or speed-curved clip, and with background stills (`lib.rs:1449-1452`). At 4K that is 15 frames (8-bit) or 7 (deep).
- **Scenario:** a 3-4 GB phone exports a reversed clip next to the open editor and gets OOM-killed. That compounds N4 (the audio mix), which is still unbounded.

**M6. A blurred background runs the clip's whole effect chain twice per frame.** *Verified.*
- `backdrop()` clones `self.effects` and appends the blur (`concat-render/src/plan.rs:463-471`).
- **Scenario:** a vertical export with a "Blur" background over a clip with a glow and a LUT costs twice the effect time per frame, plus a full-frame Gaussian. On WARP or GLES that doubles H1 and M3.
- **Fix:** blur the effected picture once, then reuse it for both.

#### LOW

- **L1. The MediaCodec first-picture probe proves a codec once per process on one file.** *Reasoned.*
  - `Verdicts` is keyed by decoder name only (`hardware.rs:361-381`), probed on whichever file came first (`decode.rs:1248-1281`).
  - A later stream that wedges the wrapper (another profile or level, or session pressure) spins inside FFmpeg with the reader's lock held. `STUCK_AFTER` (`feed.rs:68`) guards only EAGAIN rounds, not a wedge inside `send_packet`.
  - The probe also opens a session outside the seat budget, and a wedged probe leaks its thread, file and session for the life of the process.
  - The verdict mutex is held across the 2 s wait (`hardware.rs:369-381`), so up to 2 s per codec on first open (suspected acceptable).
- **L2. `next_frame` can spin without counting.** *Suspected; depends on wrapper behaviour.* When `send(End)` returns EOF it does not count toward `STUCK_AFTER` (`feed.rs:96`). A codec that answers EAGAIN to receive after draining would busy-loop forever. libavcodec's contract says it shouldn't.
- **L3. Transition knobs are neither clamped nor sanitised.** `Package::resolve` passes set values through (`concat-effects/src/catalogue.rs:655-671`). `Clip::tidy` touches only `duration` (`concat-project/src/model.rs:2032-2034`). The current shaders have fixed loop bounds (verified: blur-dissolve, whip-pan, zoom-blur), so this costs odd pictures, not hangs. A community transition with a knob-bounded loop is not GPU-trialled (`trial_at` builds an effect layer only, `gpu.rs:1808-1861`).
- **L4. The monitor falls back to three CPU round trips for every transition frame whenever any layer sits above the incoming lane** (`concat-export/src/lib.rs:2182-2189`): captions or titles, i.e. most edits. Playback stutter at cuts on weak GPUs (reasoned).
- **L5. A stabilized clip's transition handle can fall outside what was measured.** The analysis margin is 1.5 s (`concat-host/src/stabilize.rs:53-55`), while an overlap's head can be half of any length (`concat-export/src/lib.rs:874`). Before the first measured time the correction is `IDENTITY` (`concat-vision/src/stabilize/path.rs:95-107`). So a 4 s cross-fade into a stabilized clip shows its first 0.5 s unzoomed and unsteadied, then pops (suspected).
- **L6.** `WGPU_DX12_COMPILER=dxc` re-opens the PATH-DLL crash that #418 fixed (`gpu.rs:861-862`, `concat/src/gpu.rs:69-70`). Intentional, but undocumented outside a comment.

### 3. Commits worth noting (verified)

- **2969e02 (MediaCodec decode):** careful.
  - Packet timebase is set so timestamps round-trip exactly for every container clock coarser than 1 µs (`decode.rs` `pkt_timebase`).
  - Colour is read off the stream, not the wrapper (`decode.rs:741-765`).
  - Flush happens only after output (`decode.rs:934-937`); I checked that `next_source` returns only after output, so the "flush before first output loses CSD" case cannot arise.
  - The filtergraph is rebuilt from the frame's real format (`decode.rs:1032-1041`), so NV12 output is fine.
  - Gaps: M1, L1, and 10-bit staying on the CPU.
- **ac8a654:** the right fix, written once, with fake codecs.
- **b18c2fe/a9b2df6:** correct for #418. wgpu 30's default really is `Auto` (`wgpu-types-30.0.1/src/backend.rs:801-803`). Both instances are pinned. It does nothing for #297.
- **ce64dae:** the plumbing is correct (`flatten.rs:136`, `lib.rs:661, 810`), and the shader params buffer is rewritten every draw and submitted immediately (`gpu.rs:2893-2900`), so different knobs on the same key don't bleed. Weakened by M2 and L3.
- **d3c2dcd:** effect Frame is 32 bytes. The transition Frame is already 16 (`concat-effects/src/shader.rs:182-189`, `gpu.rs:2813`). Consistent.
- **16a3d13 (masks):** the WGSL matches core (`gpu.rs:87-169` against `concat-core/src/mask.rs:179-217`). The feather band is floored at a pixel, so `smoothstep` never gets `low == high`.

### 4. Numbers (HEAD; counts are grep counts, test files included)

| crate | LOC | tests | ignored | unsafe | unwrap/expect | panic (non-test) |
|---|---|---|---|---|---|---|
| media | 11 413 (+1 085) | 113 | 1 | 40 | 222 | 0 |
| render | 8 685 (+962) | 101 | 0 | 1 | 104 | 0 |
| export | 4 832 (+592) | 38 | 0 | 0 | 46 | 0 |
| core | 3 547 (+650) | 60 | 0 | 0 | 26 | 2 (arena unreachable) |
| vision | 5 498 (+2 738) | 59 | 1 | 0 | 31 | 0 |

Biggest files: export `lib.rs` 3 770 (+366), `gpu.rs` 3 695 (+134), `encode.rs` 2 553, `gpu/tests.rs` 2 361, `decode.rs` 2 032 (+275).

### 5. Scores (area; the 10-15 figure is the mean of that audit's per-crate scores)

| Aspect | 10-15 | Now | Why |
|---|---|---|---|
| Code quality | 7 | 7 | feed.rs and the seats/verdicts code are clean and tested with fakes; the god files keep growing. |
| Design architecture | 7.5 | 7 ↓ | Seats have no priority (M1); the shader-or-dissolve gate (M2); recovery opens the second device (H2); still no pipeline cache. |
| Design philosophy | 7 | 7 | Honest fallbacks and honest commit text (10-bit kept on the CPU, said why), but "rare pairing" and "30 s" are untrue. |
| Maintainability | 5.5 | 5 ↓ | export `lib.rs` 3 770 and `gpu.rs` 3 695; the stale docs from 11 and 12 Oct still stand. |
| User likeability | – (~6) | 6 | Knobs, masks, backgrounds, stabilization and phone 8-bit hardware decode are real wins; HDR phone footage is still CPU; under a grade the knobs do nothing; safe-start users get exports that hang. |
| Performance | 6 | 6 | Android 8-bit decode up a lot; offset by seat starvation, WARP+FXC, doubled effects under backgrounds, three readbacks per transition frame. |
| Scalability | 5 | 5 | Audio mix memory; fixed 512 MB export pool; recompile per export; three sessions. |
| Security | 7 | 7 | All unsafe carries SAFETY; the FXC pin removes a DLL-hijack crash; knobs unvalidated (L3). |
| Testing | 7 | 6.5 ↓ | Good unit fakes; but the MediaCodec device test is never run; no chained-transition test; no knob test through `composite_treated`; heavy GPU suites skip Windows/WARP, exactly where H1 lives. |
| Documentation | 5 | 5 | New code well commented; `DEVICE_WAIT`, `MAX_SIDE`, CPU-fallback and prefetch docs still false. |

### 6. Unverified or not checked

- MediaCodec behaviour on real phones: the only evidence is the commit text.
- Whether a VFR clip's 61.219 fps rate ever becomes a timeline rate.
- x264 ABR quality was not measured.
- FXC compile times on real (non-WARP) Windows GPUs (#297).
- concat-vision enhance (03c8fd6) was not read.
