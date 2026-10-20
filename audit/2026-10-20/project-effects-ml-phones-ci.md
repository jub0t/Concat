## project, effects, speech, vision, Android, iOS, CI and release: HEAD ce64dae (2026-10-20)

Baseline: audit/2026-10-15 (HEAD ab21457). Read-only. No cargo, no builds. Evidence = source at ce64dae (`git show ce64dae:<path>` where the working tree differs), CI/release via `gh` (read-only), and the **shipped v0.2.8 APK**, downloaded and inspected by script (ELF headers, binary manifest). Things not run on a device are marked *unverified*.

Note for the coordinator: during this audit `main` moved to baaeb2a (locale fix) and the working tree holds **uncommitted** edits to `studio.rs`, 25 locales and `media-pane.slint` (someone is fixing N4/N5 below live). Findings cite ce64dae, not the working tree.

---

### 1. Re-verification of the 10-15 findings

| # | 10-15 finding | Status at ce64dae | Evidence |
|---|---|---|---|
| 1 | Release notes' previous tag | FIXED (holds) | 0.2.8 notes span 0.2.7..0.2.8 |
| 2 | iPhone Import dead (#332) | FIXED in code, **unverified on device** | 8dfa7bb; `concat/src/ios.rs` (660 lines) installs PHPicker + UIDocumentPicker, `adopt` :516. Simulator smoke never opens a picker. #332 still open |
| 3 | Phones not built on push | PARTLY | push trigger added `mobile.yml:77`; but `cancel-in-progress` for push `mobile.yml:92` → of the last 60 main runs since 16 Oct: **50 cancelled, 7 failed, 2 succeeded** (both on the 0.2.8 tag commits 9f0cf73, 79116bd). Main is effectively still not phone-built |
| 4 | Windows heavy GPU suites skip on WARP as pass | UNCHANGED | `concat-render/src/gpu/tests.rs:41-47` (`heavy_gpu`). Effects still never run through D3D12 in CI |
| 12 | Phone that fails to start vanishes; iOS plist keys | PARTLY | plist keys FIXED `src/scripts/ios-app.sh:99-100`; Android still `return`s silently `concat-android/src/lib.rs:519-521` |
| 13 | `locales.py --check` passes with missing keys | UNCHANGED | `scripts/locales.py:193-194` prints `missing:` without `failed = 1` |
| 14 | CI internals in release notes | PARTLY | caveats section new (`.github/release-notes.sh:84-95`), but 0.2.8 notes still lead with "No effect cards are drawn on a software adapter, which froze CI's API 30 emulator" and "the lock covers one far byte" |
| 15 | DirectML asked of CPU ORT | FIXED | e461750; `concat-vision/src/runtime.rs:183-190`, `concat-speech/src/lib.rs:28-36` |
| 16 | Intel Mac ORT minimum | PARTLY | ORT now `load-dynamic`, lazily opened `build-app.yml:313-330, 640-646`, `runtime.rs:64-85`. But see N2: FFmpeg dylibs in the same bundle need 14.0. #372 open, no reporter confirmation |
| 17 | `--smoke` stops before the window | UNCHANGED | `concat/src/lib.rs:2362-2385` exits before `app.run()` (:2419); `smoke-windows.ps1:17-18` still claims "through the first frame" |
| 18 | ISA guard | PARTLY | SDE now also runs `--whisper-cpu` under `-snb` `smoke-windows.ps1:190`; Ivy Bridge still `--version` only (:182). Allowlist widened for MSVC STL AVX2 `isa-allow.txt` (79116bd) - explanation plausible (dispatched `vector_algorithms`), counts hard-coded with margin; `ggml-cpu-ivybridge.dll` row added without a measured count |
| 19 | Bundle workflow tag-only | UNCHANGED | `build-app.yml` `on: workflow_call` only |
| 20/H2 | Windows unsigned; nothing stops it | UNCHANGED (now disclosed) | v0.2.8 log 03:23:05 "No signing certificate: the build is unsigned."; `build-app.yml:1036-1037` exits 0. Notes say so (`release-notes.sh:85-86`) |
| 21 | Dependabot cargo group red | UNCHANGED | #411 (21 updates) open; its phone run 39180952703 failed |
| 28 | Concurrency cancels main | PARTLY | `ci.yml:32` no longer cancels in progress, but GitHub keeps one pending run per group, so queued main runs are still cancelled: **22 of last 60** CI runs on main cancelled |
| 29 | `ubuntu-latest` | FIXED | 94d4777; no `ubuntu-latest` in `.github/workflows` |
| §7 | smoke exit/trap/exec bit | FIXED (holds) | |
| – | Phone downloads without digest | UNCHANGED | `src/scripts/ffmpeg-mobile.sh:39-40`, `sherpa-mobile.sh:42, 60` (prebuilt native code from k2-fsa, unverified, into the shipped APK/IPA) |
| – | Cards skip with no adapter | WORSE in coverage | d9f7e08 now also skips on *software* adapters (`concat-host/src/cards.rs`), so Android emulators never compile the effect shaders at launch |
| – | AddClip takes whole media | UNCHANGED | `concat-project/src/commands/clips.rs:24-46` |
| – | Speed curve grows over neighbour | UNCHANGED | `commands/properties.rs:309-330`: duration recomputed, no ripple, no overlap check |
| – | All project tests in one file | WORSE | `concat-project/src/lib.rs` 4 533 → **5 157** lines, 136 of the crate's 150 tests |
| H1 | main red, Linux tests skipped | RECURS | main red again since 08:39Z today: 6599b5a build timed out at the 25-min cap (`ci.yml:160`), then be60358/a1fd411/f2d0637 failed `locales.py --check` (`ci.yml:141`) after f121747 removed `sounds.*` UI without its keys - clippy (:151) and tests (:168) skipped. 17-18 Oct: 22 consecutive failures (fmt, locales, tests). **28 failed + 22 cancelled of 60** main CI runs since 16 Oct; greens cluster on release commits only |
| H3 | Android check never ran app code | PARTLY FIXED | 78e4be1/766fd4e: emulator now installs an **x86_64 build of the same source** and it really starts (v0.2.8 API 30 & 35 logs: "Concat 0.2.8 starting", ResumedActivity = app). The shipped arm64 APK is still never started; the x86_64 test build links sherpa's ORT, not the shipped one, so the real-device ORT start path is still untested. Notes say so (`release-notes.sh:88-89`) |
| H4 | Intel Mac 12.0-13.2 crash | PARTLY | see 16 and N2 |
| H5 | Activity recreation kills app | FIXED (verified in shipped manifest) | 887a034, `concat-android/Cargo.toml:74`; shipped APK `configChanges=0x50003ff4` = all 13 flags. Still missing `mcc|mnc|touchscreen|colorMode|grammaticalGender` (rare). Second-`android_main` guard moot: the loop end calls `leave()` → `process::exit` (`concat/src/lib.rs:2419-2434`) |
| M1 | Import crash after process kill / provider error | PARTLY | SecurityException FIXED (724a256, `ConcatFiles.java:355-364` catches `RuntimeException`). Dex-loaded Fragment restore after process death UNCHANGED (`lib.rs:154-195`, `ConcatFiles.java:221-225`: system restores `app.concat.editor.ConcatFiles` with the activity's class loader → ClassNotFoundException). *Reasoned, unverified* |
| M2 | Android projects deleted on uninstall | UNCHANGED | `lib.rs:37-38` HOME = external_data_path; imports copied to `getExternalFilesDir/Imported` (`ConcatFiles.java:282`), no de-dup (`unique()` :325-340 makes "name (2)") |
| M3 | 0.2.7 notes false claim | FIXED | v0.2.7 body now starts "Corrected on 16 October" |
| M4 | Shipped .ipa never launched; prebuilt minimums | PARTLY | iOS min-OS check added `mobile.yml:420-425` (passes: "2 Mach-O files... every file runs on 15.0"); still a separate sim build on newest runtime only; #294 open |
| M5 | Nobody knows if main builds for phones | PARTLY | see 3 |
| M6 | Vision: no CPU retry on accelerator run-time failure | UNCHANGED | `runtime.rs:94-110, 140-143`; NNAPI/CoreML never in CI |
| M7 | No offline shader portability check | UNCHANGED | `concat-effects/Cargo.toml` naga `wgsl-in` only |
| L1 | `pow` negative base | UNCHANGED | `packages/concat.light-leak-flash/effect.wgsl:17` |
| L2 | No `permissions:` | UNCHANGED | only `models.yml:45`, `release.yml:120`, `nix.yml:40` have one |
| L3 | Tag re-pointed after a failed run | REPEATED | v0.2.8 run 39180271851 on 9f0cf73 failed (Windows ISA scan), tag moved to 79116bd, run 39188732930 passed |
| L4 | Consolidation orphans renders | UNCHANGED (no commits) | `concat/src/studio/consolidate.rs` |
| L5 | Font mmap SIGBUS | UNCHANGED | `concat-text/src/lib.rs:654` |

Scorecard of 10-15 items: fixed 6 (1, 15, 29, H5, M3, §7), partly 14, unchanged 13, worse/recurring 4 (H1, project test file, card coverage, L3).

---

### 2. CI and release status (real, 20 Oct ~10:35Z)

- **v0.2.8**: tag 79116bd, built 19 Oct, draft published 20 Oct 08:18Z. All 16 jobs green incl. CI-in-release, 5 desktop bundles, Android arm64+x86_64, two Android smokes, iOS arm64 + simulator smoke. Downloads so far: setup.exe 244, APK 163, macOS arm64 45.
  - Windows: **unsigned** (log). macOS: ad-hoc, not notarised (annotation). Android: signed with the real key (V3 signer SHA-256 9233...6e2e, same as 0.2.7).
  - **Release does not fail when unsigned** (`build-app.yml:1036-1037`); it only adds a caveat (`release.yml:172`, `release-notes.sh:84`). Same for macOS min-OS violations (warnings only, N2).
  - Android API 35 smoke passed with a warning "window in focus is ... Application Not Responding: nexuslauncher" (job 117867497573): a system ANR dialog over the app is accepted.
- **main**: red since 08:39Z today (see H1 row). CI on main since 16 Oct: 8 success / 28 failure / 22 cancelled. Phones on main: 2 success / 7 failure (all 4 inspected: API 30 emulator froze, 30-min timeout, before d9f7e08) / 50 cancelled.
- **Phone workflows on PRs**: yes, `pull_request:` (`mobile.yml:76`); owner-branch PRs complete (39268016072 etc.); first-time contributors' PRs sit at `action_required`.
- **Does CI prove each platform starts?**
  - Windows x86_64: imports resolve, `--version`, `--smoke` (stops before the event loop), SDE Ivy Bridge `--version`, SDE Sandy Bridge `--whisper-cpu`. **No window, no first frame, no real GPU, no D3D12 effect compile.** #339/#365-class failures invisible. Windows ARM not shipped (#326).
  - Android: x86_64 emulator build really starts on API 30/35 for 60 s. **Shipped arm64 never started; no real device; no 16 KB-page device (N1).**
  - iOS: simulator build really starts (newest runtime only). **Shipped .ipa never launched.**
  - macOS: `--smoke` on runners (15.x); nothing on 12/13/14 despite declaring 12.0 (N2).
- **Model mirror never exists**: `models.yml` has **never run** (no runs) and `gh api .../releases/tags/models-v1` → 404. Every model download's first source (`concat-host/src/models.rs:29-42`) is a dead URL; the app always falls through to third parties (GitHub releases of PeterL1n, danielgatis, k2-fsa; Hugging Face at pinned commits). Integrity holds (digest required, `models.rs:311-321`, test :495), availability does not. `HF_REPO = ""` (:72) so China users get only hf-mirror of HF upstreams.

---

### 3. New findings

#### HIGH

**N1. The shipped Android app library is 4 KB-aligned: it cannot load on a 16 KB-page phone.** *Verified on the v0.2.8 APK.* ELF PT_LOAD alignment: `libconcat_android.so` **4096**; `libonnxruntime.so`, `libsherpa-onnx-c-api.so`, `libc++_shared.so` 16384. Only FFmpeg gets `-Wl,-z,max-page-size=16384` (`src/scripts/ffmpeg-mobile.sh:92`); the Rust cdylib gets nothing (no RUSTFLAGS/link-arg in `mobile.yml` or `mobile-env.sh`). Scenario: a phone running a 16 KB kernel (Pixel 8/9/10 "16 KB" mode, devices launching on Android 16+) → `dlopen` refuses "program alignment (4096) cannot be smaller than system page size (16384)" → app closes at launch with nothing shown (lib.rs never runs); on Android 16 compat-mode devices, a warning dialog at best. Google Play has required 16 KB alignment since Nov 2025, so this is also a hard blocker for any Play listing. Fix: `-C link-arg=-Wl,-z,max-page-size=16384` for `aarch64-linux-android`, plus a CI check over every `.so` in the APK (same shape as `check-min-os.sh`).

**N2. macOS bundles declare 12.0 and ship Homebrew dylibs built for 14.0/15.0; the check only warns, and 0.2.8's notes claim Intel Macs open on 12.** *Verified in CI annotations; launch effect unverified (no macOS 12-14 runner).* v0.2.8 run 39188732930: arm64 bundle - libavfilter, libavdevice, libx264, libdav1d, libSvtAv1Enc, libvmaf, libmp3lame, libmpg123, libssl, libcrypto "needs 15.0, newer than the declared 12.0"; Intel bundle - same set "needs 14.0" (libssl/libcrypto 15.0). `check-min-os.sh:9-12` itself says dyld "loads it, and the process dies on the first symbol the older system lacks"; `build-app.yml:640-654` runs it without `--enforce` ("A warning for now"). `build-app.yml:96-97` says "Apple silicon Macs all run newer than this" - false (M1 Macs run 11-15). Release note "🐛 Intel Macs open on macOS 12 again (#372, #333, #342)" is unproven: the ORT crash is removed, the next missing symbol may be FFmpeg's. Fix: build FFmpeg for the floor (as phones already do) or raise `MACOSX_DEPLOYMENT_TARGET` to 14/15 honestly; then `--enforce`.

**N3. Auto-captions start seconds early (#438) and nothing in the pipeline addresses it.** *Verified by reading; reporter's numbers untrusted but consistent with the code.* `concat-speech/src/transcribe.rs:481-497`: greedy whisper with no VAD, no token timestamps, no split-on-word, no max segment length; `:544-545` takes segment timestamps raw. Whisper's segment start is the previous segment's end, so every pause is swallowed into the next caption (reported 1.1-7.1 s early). Captions are the most-used AI feature in this class of editor. Fix: whisper.cpp's Silero VAD (or an energy VAD on the 16 kHz buffer already in hand) to trim leading silence per segment, `set_token_timestamps(true)` and re-anchor each caption on its first token; a regression test with a synthetic tone-gap file.

**N4. CI is not a signal on main: 50 of 60 phone runs cancelled, 50 of 60 CI runs red or cancelled.** *Verified via gh.* Causes: `mobile.yml:92` cancels in-progress push runs (a 50-60 min build vs pushes every 5-20 min); `ci.yml:31` group keeps one pending, cancelling the rest; cheap gates first (`ci.yml:136` fmt, `:141` locales) abort the job before clippy (:151) and tests (:168) - the H1 shape again today (f121747 dropped `sounds.*` without regenerating locales, 4 consecutive reds); the 25-min build cap (`ci.yml:160`) fails a cold cache (6599b5a, plus a cache-save tar error). Releases pass only because `release.yml` re-runs CI on the tag. Net: nobody can say which main commit broke a phone. Fix: run fmt/locales/models in their own job; `if: !cancelled()` on clippy/tests; don't cancel push runs on main (or run phones on a schedule against the latest main and let it finish); raise the build cap or prime the cache.

#### MEDIUM

**N5. ce64dae's transition knob: one undo step and a full model rebuild per slider tick.** *Verified at ce64dae.* `set_transition_param` uses `self.apply` (`git show ce64dae:src/crates/concat/src/studio.rs:4306`), not the gesture-folded path the effect knobs use (`studio.rs:5593-5611`); `Param.changed` fires on every move (`ui/inspector/controls.slint:421`). Dragging Wipe softness 0→1 = dozens of undo steps and an info log line each. Publishing creates a fresh `VecModel` every time (`:9044`), so the slider row is rebuilt under the finger mid-drag (worst on a phone). An uncommitted working-tree fix switches to `apply_within("transition-knob:{clip}:{key}")`, but never calls `end_gesture`, so two separate drags of the same knob with nothing between fold into one step (`concat-project/src/editor.rs:277-283`). No test covers either.

**N6. Transition params bypass the NaN guard; a non-finite value makes the project unopenable after save.** *Verified by reading.* `Command::has_non_finite` checks only `tr.duration` (`concat-project/src/commands/mod.rs:1092-1096`), not `tr.params` (filters' params are checked, :989-990). serde_json writes NaN as `null`; `from_document` (`doc.rs:170-173`) fails the whole parse on `null` for an `f64` and returns None → "nothing to open". The UI path refuses non-finite (`studio.rs` `is_finite` check), so the reachable callers are Rust API users/tests; latent but one line to fix. Also no render-time clamp: `concat-effects/src/shader.rs:1673-1685` writes any value from the file into the uniform, so a hand-edited or API-written `direction: 1e9` reaches the shader unvetted (install vet only tries min/default/max).

**N7. Project files written by ce64dae+ lose their transition knobs silently in 0.2.8.** *Verified by reading.* `Transition` has no `deny_unknown_fields` and `extra` passthrough is top-level only (`doc.rs:358-362`), so 0.2.8 (desktop or a phone that has not updated) drops `params` on read and the next autosave writes the project without them; `DOCUMENT_VERSION` stays 1 (`doc.rs:41`). Backward read is fine (`#[serde(default, skip_serializing_if)]`, `model.rs:1311-1315`). Acceptable for an optional knob, but it is the first field whose loss changes the picture; there is no per-object passthrough or minor-version marker to warn.

**N8. Model mirror does not exist and has never been filled.** See §2. `models/manifest.toml:8-14` promises "stays reachable... under one roof"; reality: every download depends on five third-party hosts, two of them personal GitHub releases (`rvm_mobilenetv3_fp32.onnx`, rembg `v0.0.0`). One deleted release = a feature gone for every user, with a 404 retry first.

**N9. Phone native dependencies are fetched without digests into signed release artifacts.** `ffmpeg-mobile.sh:39-40` (ffmpeg.org tarball), `sherpa-mobile.sh:42, 60` (prebuilt `.so`/xcframework from GitHub). Desktop downloads are digest-pinned (`scripts/ci/deps.lock`, `fetch.sh`); phones are not, and the APK is then signed with the real key. A compromised upstream release ships under Concat's signature to 16 000+ Android users.

**N10. Captions ignore the speed curve and the clip moving during the run.** `concat/src/panes/captions.rs:496` (window = duration × mean speed), `:523` (`speed: clip.speed`), `:211-212` (place at `start + t / speed`). On a ramped clip the captions drift by the integral difference; a clip moved or trimmed during a multi-minute transcription gets captions at its old position. *Reasoned.*

**N11. API 35 smoke accepts a system ANR over the app; nothing checks a picture.** `smoke-android.sh:322-324` turns "focus is not the app's" into a warning; the screenshot (`:102`) is collected, never inspected. v0.2.8's pass had the launcher's ANR dialog on top. A black or frozen first frame passes.

#### LOW

- **L-a** `AddClipsInSequence` non-magnetic can reverse order: each item takes `nearest_free_start` from the previous end (`commands/clips.rs:68-118`, `placement.rs:42-66`), whose candidates include `clip.start - duration` of the clip just placed. Lane with X at [26,60], drop A(5 s), B(5 s) at 20: A→20, B→15. 38056f1's "end to end on one lane" holds only when the gap is free. *Reasoned.*
- **L-b** SRT import is all-or-nothing: one malformed cue (`srt.rs` `parse`, `fraction.len() > 3`, missing blank line) rejects the file; `<i>`, `{\an8}` tags become literal caption text.
- **L-c** Voice cloning ("Clone from a sample…", `locales/en.json:550`) has no consent prompt and no watermark (`THIRD_PARTY_NOTICES.md:158-169` acknowledges both). Product/abuse risk, not a licence breach.
- **L-d** `orientation = "portrait"` (`concat-android/Cargo.toml:80`) and targetSdk 35 (:54): Android 16 ignores orientation locks on large screens for targetSdk 36; the phone shell will be forced into landscape on tablets/foldables once the target is bumped.
- **L-e** `ConcatFiles.done` uses `getExternalFilesDir(null)` unchecked (`ConcatFiles.java:282`); null on unmounted external storage → `mkdirs` on a relative path fails → the pick returns nothing, silently.
- **L-f** Card thread starved a 4-core emulator to a system freeze (d9f7e08 message); the fix hides it on software adapters only. Low-end real phones run the same 100+ pipeline compiles at launch. *Unverified on device.*

---

### 4. Effect package format, against the CapCut report

Verified at HEAD: one WGSL file + `effect.toml` + `fixtures.toml` + optional `.cube`; ≤8 passes (`manifest.rs:453`); unknown manifest fields denied; bindings fixed by the host; no image/sequence/video inputs, no mask input, no frame history, no audio/beat; unbounded loops refused at load (`shader.rs:1067-1113`), GPU vet at install. ce64dae delivered half of the report's Step 0: transitions now carry `params` (`model.rs:1311-1315`, threaded through `concat-export/src/lib.rs:384-387, 661, 810`), but not `keys` (knobs can't be keyframed), and no `clip_duration`/`frame_index`/`seed` in either uniform (`shader.rs` `TRANSITION_HEAD` :181: size, time, progress only). The sandbox is sound for security; it is the reason packages can't make the motion/flash/overlay effects users want. M7 (no offline HLSL/GLSL cross-compile) becomes mandatory before any binding is added.

---

### 5. Numbers (all `.rs` lines; tests = `#[test]`)

| crate | lines (10-15) | tests | unsafe | unwrap+expect | notes |
|---|---|---|---|---|---|
| concat-project | 12 899 (10 685) | 150 (121) | 0 | 446 | `lib.rs` 5 157 lines, 136 tests |
| concat-effects | 7 355 (7 256) | 72 | 0 | 139 | 127 packages, 1 966 WGSL lines |
| concat-speech | 3 504 (3 411) | 22 | 5 | 23 | `tts.rs` 1 504 |
| concat-vision | 5 498 (2 760) | 59 (31) | 0 | 32 | +stabilize |
| concat-android | 595 + 373 Java | **0** | 9 | 9 | |
| concat/src/ios.rs | 660 (new) | 0 | many `msg_send!` | – | |

---

### 6. Scores (1-10; previous from the 10-15 crate rows / overall table)

| Aspect | 10-15 | Now | Why |
|---|---|---|---|
| Code quality | ~7.3 | 7 | Careful, commented, tested commands; but ce64dae shipped the knob without the gesture path or NaN guard every sibling has (N5, N6) |
| Design architecture | ~8 | 7 ↓ | Full-snapshot undo and doc migration still good; transition params bolted on without keys or passthrough (N7); captions placement ignores the curve (N10); package sandbox sealed off from motion/masks |
| Design philosophy | ~7 (overall 5) | 6 | Honest "Before you install" caveats are a real step up; but gates that only warn (min-OS, signing, foreign ANR), merging on red, tag re-pointed again |
| Maintainability | ~6.5 | 6 | project test file +624 lines; studio.rs keeps absorbing features (knobs, iOS); Android 0 tests |
| User likeability | 6 | 6 | iOS Import, Android config changes, transition knobs, Intel ORT lazy: real gains. Captions early (N3), unsigned Windows, 16 KB phones can't open (N1) |
| Performance | ~5.8 | 6 ↑ | whisper leaves two threads, AVX2 back on capable CPUs, cards pre-shipped; nothing measured on a phone |
| Scalability | ~5.6 | 5 | per-tick republish of whole models; caption/undo snapshots of whole project; mirror absent |
| Security | ~7 | 6 ↓ | Model digests fail closed (good); phone deps unverified into signed APK (N9), Windows unsigned again, no `permissions:` in three workflows, no render-time clamp (N6) |
| Testing | ~5.7 | 5 | Android emulator smoke is now real (+), iOS min-OS check (+); but shipped arm64/ipa never started, WARP skips heavy GPU suites, main mostly red/cancelled, new knob path untested, Android crate 0 tests |
| Documentation | 6 | 6 | Notes corrected and caveated; still false: `smoke-windows.ps1:17-18`, `build-app.yml:96-97`, `models/manifest.toml:8-14`, "Intel Macs open on macOS 12 again" |
| **CI and release** | 4 | **5 ↑** | Real Android launch (x86_64), phones in release, min-OS scan, honest caveats. Held back: 50/60 phone and 50/60 CI main runs not green, warnings don't gate, unsigned release, tag moved, mirror never run, no 16 KB check |

Phone launch checks: Android 2 → 5 (real launch, wrong ABI); iOS 7 → 7.

### 7. Top fixes, in order
1. 16 KB link flag for the Android cdylib + an alignment check in `mobile.yml` (N1). An hour.
2. Decide the macOS floor honestly: build FFmpeg for 12, or declare 14/15; then `check-min-os.sh --enforce` (N2).
3. Captions: VAD/leading-silence trim + token timestamps, with a gap test (N3, #438).
4. Split lint gates into their own job, stop cancelling main push runs, raise the cold-cache cap (N4).
5. Commit the knob gesture fix with an `end_gesture` timer like the effect knobs; add `tr.params` to `has_non_finite`; clamp at `lay_params` (N5, N6).
6. Run `models.yml` once (N8); pin phone downloads in `deps.lock` (N9).
7. Fail a tag build without a Windows certificate, or keep the caveat and stop calling the release a pass.
