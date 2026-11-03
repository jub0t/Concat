## Engine and export: media, render, export, host playback. main eda5e0af, 2026-11-03

Labels: **Verified** means I read it at the line, or ran it. **Reasoned** means I followed the code path but ran nothing. **Suspected** means the reason is given and it needs a check on a machine.

What was run:
- `cargo fmt --check` and `scripts/models.py --check`.
- `ffmpeg -encoders` on the bundled BtbN LGPL build.
- A release build of the workspace on Windows. It compiled every library and linked `concat-cli.exe` and `concat-mcp.exe`.

No tests were run.

Since 3e651f4 these crates have about +9 000 / −2 700 lines:
- concat-host +3 060 / −240.
- concat-media +2 359 / −185.
- concat-export +1 102 / −1 152.
- concat-render +1 012 / −24.
- concat-effects +915 / −106.

The work falls into three groups:
- The codec move to platform encoders: `32945bc`, `536a416`, `21af697`, `c415037`, `fa79f78`, `5ab7fd1`.
- Speed:
  - Enhance on the GPU: `aa38b9d`, `7db37f2`.
  - The encoder on its own thread: `9e69bbc`.
  - Scrubbing: `75919f5`.
  - Sound after a cut: `e0111c0`.
  - Idle memory: `f19fda3`.
- The #621 transition rewrite, `f2a2608`, committed as work in progress.

### 1. The 24 October findings

| Finding | Status | Evidence now |
|---|---|---|
| H7 / O3: export recovery opens a fresh device | **Unchanged** | `recovered()` still calls `WgpuCompositor::new()` on the first loss (`concat-export/src/lib.rs:1516-1520`) |
| E-M1: reader slack counted in source frames | **Unchanged** | `claim_cost` still forgives `prefetch::AHEAD` source frames (`concat-media/src/pool.rs:359-367`) |
| E-M2: sound seeks never check where they land | **Fixed** (#535, `b5042b5`) | Both readers seek on the sound's own stream, backwards, to the packet at or before the cut. Two new fixtures cover it: an MPEG-PS file with its sound behind the picture, and a TS file with sparse keyframes |
| E-M3: phone software rerun may cap below 4K | **Not rechecked** | Still needs a device |
| E-M5: GLES waits, default bitrate, 512 MB pool | **Partly** | x264 is gone, so the old default-bitrate finding is moot. H.264 now goes to a platform encoder steered by bitrate (`9e69bbc`, `32945bc`). The pool was not rechecked |

### 2. New findings

#### HIGH

**X-H1. The transition rewrite is on `main` half-done: its old tests are deleted, it has no new ones, and it changes every saved transition.** *Verified.*

`f2a2608` is titled `wip(transitions)` and says "stage 1 … the new model's own tests are still to come".
- It deleted 16 export tests of transition behaviour from `concat-export/src/lib.rs`: cross-fades centred on the cut, short handles, held frames, push/zoom/wipe/fade, and keyed clips.
- It ignored two more (`lib.rs:2953`, `lib.rs:3062`). One of them, `the_monitor_draws_wipes_and_fades_as_the_export_does`, is the only check that preview and export agree on transitions.
- `b8b6528` names a failing export-suite test as "#621's".

The commit changes what existing projects look like:
- "A transition still set on a cut the old way is made one centred on that cut." The deleted `a_short_handle_leans_the_overlap_after_the_cut` covered the old rule, where a transition with little footage past the cut leaned to one side. That rule is gone.
- A push over a held outgoing frame used to become a dissolve (`a_push_over_a_held_outgoing_frame_becomes_a_dissolve`, also deleted). It now runs the package over a frozen frame.

Nothing in the window places a transition as a clip yet, so users can't reach the new model. The change to how old projects render, however, reaches every project with a transition.
- **Scenario:** the next release is cut from `main`. Every project with a transition renders differently, and nothing tests what it now renders.
- **Fix:** either finish stage 2 with its tests before a release, or keep the old resolve path until then.

#### MEDIUM

**X-M1. HEVC is offered where nothing can encode it. On Windows and Linux, a stalled HEVC export can't continue in software.** *Encoder selection verified; the effect on a real machine is suspected.*
- Without x265, the HEVC list is `hevc_nvenc, hevc_qsv, hevc_amf, hevc_mf` on Windows and the three vendor encoders alone on Linux (`concat-media/src/encode.rs:220-221`).
- `Family::MediaFoundation` counts as software (`encode.rs:547-552`), and `encodable_among` answers yes for a software encoder "without opening anything" (`encode.rs:603-613`). So on Windows the export sheet always offers HEVC.
- FFmpeg's `hevc_mf` is not told to use hardware (no `hw_encoding` in `encode.rs`), so it asks for a software HEVC encoder from Media Foundation. Most Windows installs don't have one: it comes with Microsoft's HEVC Video Extensions. This part is *suspected* and needs one Windows machine with no GPU encoder to confirm.
- A stalled NVENC/QSV/AMF HEVC export reruns with `software_only` (`concat-export/src/lib.rs:2078-2092`). `allowed_attempts` then filters out the vendor encoders (`encode.rs:809-818`), leaving only `hevc_mf` on Windows and nothing on Linux.
- The same question applies to H.264 on Windows N editions without the Media Feature Pack: `h264_mf` is the only CPU H.264 encoder there.
- **Scenario:** a laptop whose GPU has no HEVC encoder, or a desktop whose NVENC stalls (#491's pattern, now on Windows). The user picks HEVC and the export fails, either at once or after rendering part of the file.
- **Fix:**
  - Open `hevc_mf`/`h264_mf` once before offering them, as `encodable_among` already does for the system encoders on phones.
  - Consider `libkvazaar` (BSD-3, already in the BtbN LGPL build) as a software HEVC encoder for the rerun.
  - Let the sheet say which encoder an export will use.

**X-M2. E-M1 carried.** Fast sources still open extra readers on plain playback.

#### LOW

- **X-L1. The `9e69bbc` commit message says a stalled chip "still hands the export to x264".** x264 was removed four commits later (`32945bc`). The code follows the new lists, so only the history is misleading.
- **X-L2. H.264 on Linux without a GPU is OpenH264, steered by bitrate** (`encode.rs:217-218`, `encode.rs:1007`). Expect larger or softer files than x264 made. Neither `h264_vaapi` nor `hevc_vaapi` is in the Linux lists, so AMD and Intel GPUs on Mesa encode H.264 on the CPU and HEVC not at all. The notices say plainly that BtbN's OpenH264 carries no Cisco patent licence (`THIRD_PARTY_NOTICES.md:25-28`).
- **X-L3. Ten-bit H.264 is gone.** The sheet handles it: `ten_bit_available()` gates the switch (`concat/src/panes/export.rs:424-431, 647`).

### 3. Commits reviewed

| Commit | Verdict |
|---|---|
| `32945bc` platform H.264/HEVC | The lists are coherent, and the notices are honest about patents. Open: X-M1, X-L2 |
| `9e69bbc` encoder thread | Bounded `sync_channel(ENCODER_QUEUE)` and the encoder opened on its own thread (`lib.rs:1940-2035`). Cancel and join are handled. Measured 4K60 11.3 s → 4.7 s on an M5. The message is stale (X-L1) |
| `f19fda3` idle trim | Benign: caches trimmed after 5-10 s idle, then the allocator's own trim on each platform (`concat-host/src/memory.rs:60-90`). 590 → 511 MB measured |
| `e0111c0` sound after a cut | A split half borrows the parent's decoded samples instead of decoding again. A real fix for multi-second silences |
| `b5042b5` #535 | Fixes E-M2, with fixtures |
| `75919f5` scrub (#472) | Shows the nearest decoded picture at once, and the exact frame once the playhead rests. `scrub_feel` is an ignored measurement, not a check |
| `aa38b9d` / `7db37f2` / `94f4255` Enhance | Own compute shaders: 27 s → 4.2 s a 1080p frame. CoreML was made the default, then reverted by timing both engines in the running app. Only the shown span is enhanced, and byte-identical frames are reused |
| `c63c7ec` #548 | Samples at level 0 so a blur's loop has no derivative. Correct for D3D12/FXC |
| `f2a2608` #621 | X-H1 |

### 4. Numbers

| Crate | `#[test]` now (24 Oct) | Ignored now |
|---|---|---|
| media | 158 (143) | 3 |
| render | 108 (101) | 1 |
| export | **25 (46)** | 3 |
| host | 161 (137) | 3 |
| effects | 74 (72) | 0 |
| core | 64 (64) | 0 |

Ignored tests across the workspace went from about 1 to 15. Thirteen are probes, benchmarks or need a device or model. The two #621 ignores are the ones that matter.

Big files:

| File | Lines (change since 24 Oct) |
|---|---|
| export `lib.rs` | 4 279 (−79; the hand-written transitions are gone) |
| `gpu.rs` | 3 769 |
| `encode.rs` | 3 326 (+513) |
| `decode.rs` | 2 517 |
| `pool.rs` | 2 353 (+481) |
| `playback.rs` | 2 204 (+425) |
| `audio.rs` | 2 050 (+349) |

### 5. Scores

| Aspect | 24 Oct | Now | Why |
|---|---|---|---|
| Code quality | 7 | 6.5 ↓ | Careful, measured fixes (#535, the sample borrow, the encoder thread). But a rewrite is committed as WIP with its tests deleted |
| Design architecture | 6.5 | 7 ↑ | One transition path through packages instead of five hand-written ones, and the encoder decoupled from drawing. The rerun ladder no longer has a software encoder at its foot for HEVC |
| Performance | 7 | 8 ↑ | Enhance 6×, export 2.4× at 4K60, instant sound after a cut, scrubbing that answers at once, less idle memory |
| Scalability | 5 | 5 | Pool and seats unchanged |
| Testing | 7 | 6 ↓ | +15 media and +24 host tests with real fixtures, but −21 export tests and none for the new transition model |

### 6. Must fix before a release

1. **X-H1.** Finish #621 with tests, or keep the old resolve path for the release.
2. **X-M1 needs one Windows machine without a GPU encoder:** export HEVC, and H.264 on an N edition.
3. **H7** is still open.

### 7. Not checked

- Real exports through Media Foundation, OpenH264 or VideoToolbox's `require_sw`.
- How the transparent export (ProRes 4444, VP9 alpha) looks in other apps.
- The GPU Enhance shaders on AMD and Intel.
- E-M3 on a phone.
