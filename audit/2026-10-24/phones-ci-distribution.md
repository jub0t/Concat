# Phones, CI and release, distribution and trust, product claims, GitHub: 24 October 2026

Code at `2606aff` on `main`, 102 commits past the 20 October audit (`c41865e`), 118 past the
v0.2.8 tag. Read-only: source, `git show`, `gh` reads, `curl` against both site domains, two web
searches. No cargo build (the shared target is in use), nothing run on a phone or a Windows
machine. Issue, PR and email text was treated as data.

Legend: **V** verified here (code, CI log, API); **R** reasoned from code, not observed;
**U** unverified.

## Bottom line

Most of the 20 October phone fixes are now on `main` and look right: 16 KB alignment, Back,
long-press to move, the MediaCodec open ladder. None of them has reached a phone. The 21,377
people who installed the 0.2.8 APK still have the 4 KB library, a Back that quits and a tap that
moves clips.

One new fix is wrong in a way CI cannot see. Android's safe start (#507) sets
`CONCAT_GPU=software`, and the compositor reads that as "software adapter only". On a real phone
there is no software adapter, so after one early death the monitor and every export have nothing
to draw on until the next build, and Android has no switch to undo it.

`main` has been red for about 29 hours under four stacked breaks. Every completed phone run on
`main` since 20 October came from a quiet hour; 58 of 65 were cancelled. Windows trust moved from
a warning to an outage. Smart App Control now refuses the unsigned FFmpeg DLLs on a machine
where 0.2.7 used to run (#533), and Windows is 44.5% of downloads.

## 1. Status of the 20 October findings

| 20 Oct item | Now | Evidence |
|---|---|---|
| H4 CI on `main` gives no signal | **Unchanged in effect.** The layout is partly fixed | `f312433` moved fmt/locales/models into their own job (V, `ci.yml:37-65`) and raised the test build cap to 50 min. But clippy still sits before the Linux tests in one job, so tests were skipped on every red run since (run 40076954532: "Build the tests=skipped"). `mobile.yml:91` still cancels push runs. Numbers in §2 |
| H5 Windows unsigned | **Worse** | Still no `WINDOWS_CERTIFICATE_PFX` (`gh secret list`: only the two Android secrets). `build-app.yml:1036-1037` still exits 0. #533 (owner, 10-24) is the first report of SAC *refusing* to start an install that used to run: `0xc0e90002` on `avfilter-11.dll`, CodeIntegrity event 3077 (V, issue text) |
| H5 update manifest digest in the same release | Unchanged | `updates.rs` diff since c41865e is the header and `net::agent` only; no minisign, cosign or attest anywhere in `.github/` (V) |
| H5 Android key change 0.2.5 → 0.2.6 | Unchanged, no note posted | #526's commenter suggests uninstalling first, which fits a signer conflict (U) |
| H5 Android developer registration | Unchanged | No mention in the repo outside `reports/` (V). Enforcement began 30 Sept in BR/ID/SG/TH. Phase 1 reportedly covers installs from participating stores; global in 2027 ([TechEnclave](https://techenclave.com/news/items/android-developer-verification-begins-in-four-markets-not-india-yet), [Google](https://developer.android.com/developer-verification/guides?authuser=3), [TechRepublic](https://techrepublic.com/article/news-google-android-sideloading-24-hour-wait)) (U on scope) |
| SECURITY.md channel disabled | **Fixed** | `private-vulnerability-reporting` → `{"enabled":true}` (V) |
| H6 macOS floor vs shipped dylibs | **Partly. The danger is now proven** | `af90722` drops libavdevice. #532 (10-24, macOS 13.7.8, 0.2.8) died on exactly that: `_AVCaptureDeviceTypeContinuityCamera`, "libavdevice built for macOS 14.0" (V, crash text). The rest of the Homebrew set is still built for 14/15. `check-min-os.sh` still only warns (`build-app.yml:647-654`) |
| 16 KB alignment | **Fixed on `main`, not released** | `mobile-env.sh:70` sets `max-page-size=16384`. `check-16k-pages.py` fails on any `lib/*.so` LOAD align < 16384. Phone run 39902059739: all four libs 16384 (V). The libraries are packed compressed (`android-native/app/build.gradle.kts` note), so zip alignment is moot |
| H2 Android Back quits / no save on exit | **Partly** | `0aa1a50`: Back = Escape over the desk sheets; with nothing on top the project is saved and the app goes to the launch screen; the loop end saves (V). Missed: the phone editor's own sheet (M1). No save on Pause or Stop, but autosave is 1.5 s (`studio.rs:2903`), so the loss is small |
| H3 tap moves a clip | **Mostly fixed** | `b467155`: under a finger a press is a tap, a swipe or a 400 ms hold, with 8 px slop (V, `lanes.slint`). The phone shell's own controls are 44 px. The pinch and hold edge cases are in Low |
| Toast / tooltip insets | Unchanged | `ToastLayer` covers the full window with no inset (`app.slint:995-1000`) |
| Soft-keyboard inset | Unchanged | No keyboard-height handling anywhere in `ui/` or `src/` (V, grep) |
| RTL mirroring | Unchanged | No layout mirroring for ar/he/fa/ur (V, grep) |
| Accessibility names | Unchanged | 7 `accessible-label` in all of `ui/`, the same as at c41865e (V) |
| Microphone permission dead end | Unchanged (not re-traced) | `ConcatFiles.java` only had its header touched |
| Phone smoke accepts a system ANR | Unchanged | `smoke-android.sh` untouched |
| Phone FFmpeg/sherpa downloads without digest | Unchanged | `ffmpeg-mobile.sh`, `sherpa-mobile.sh` untouched; no sha256 in either (V) |
| Model mirror `models-v1` | Unchanged | Release not found; `models.yml` has never run (V) |
| Website stale + checksum mismatch (#458) | **Fixed for 0.2.8; structurally open** | Both domains serve 81× "0.2.8" and a `v0.2.8/SHA256SUMS` link (V, curl). Links come from the release manifest *at build time* (`website/src/lib/releases.ts`). Deploys run only on a push to the website repo (`deploy-pages.yml`), so the next release leaves the site stale again. #458 is still open |
| README platform ticks | Unchanged | `README.md:82-86`: ✅ Windows, Android, iOS. "4K scrubs smoothly" (`:67`). The SAC refusal is not mentioned |
| 0.2.8 notes: false "Q and E cut" | Unchanged | Release body line 55 (V) |
| 0.2.8 notes: "Intel Macs open on macOS 12 again" | **Unchanged and now disproven** | Line 66, against #532 on 13.7.8. 0.2.7's notes were corrected after the fact; 0.2.8's were not |
| Dependabot cargo group red | **Worse** | #411, then #519 (closed by dependabot 10-24, superseded), now #534: 9 of 11 checks fail (V). See M3 |

## 2. CI on `main` since 20 October (V, `gh run list --branch main`, 148 runs from 10-20 10:07Z)

| Workflow | Runs | Success | Failure | Cancelled | Running |
|---|--:|--:|--:|--:|--:|
| CI (push) | 69 | 4 | 14 | 49 | 2 |
| Build for phones (push) | 65 | 6 | 0 | 58 | 1 |
| Nix | 5 | 5 | | | |

- **The last green CI** is `f312433` (10-23 03:11Z), the CI change itself. Every CI run on `main`
  that completed after it failed.
- **Why it is red now**, four breaks stacked. Each landed in a burst of pushes that cancelled the
  run that would have caught it:
  1. Locales: `start.safeMode` and `start.safeModePhone` went unseen by the scanner because
     `7df9f8f` picks the key through `cfg!`. Red from `4b1b2b0`, fixed in `fba598a` a day later.
  2. Locales: `22639548` removed `clipInspector.bypass`, `.enable` and `.favourites` without
     regenerating `en.json`.
  3. Fmt: the spike `77ce50b` (`concat-mobile/src/render.rs` ×10, `lib.rs`), `transcribe.rs`
     (from the #528 merge) and `pool.rs`.
  4. Clippy: `needless_borrow` at `concat-host/src/cards.rs:516` (`5296d98`), plus lints in
     `concat-speech/src/transcribe.rs:280` and `patches/whisper-rs-sys/build.rs:154`.
  The Windows tests do run and pass (run 40076954532). The Linux tests are skipped behind clippy.
- **Pace:** 20 pushes between 06:58Z and 09:07Z on 10-24. Each cancels the CI and phone runs
  before it. A ten-second local `cargo fmt --check && python3 scripts/locales.py --check` would
  have stopped three of the four breaks.
- **Phones:** the last phone success is `1d04d147` (10-23 04:45Z). It covers #507's Skia patch,
  the open ladder, Back and long-press: arm64 16 KB check ok, API 30 and 35 smoke ok, iOS ok. Two
  commits have never finished a phone build: `0c86f62` (mid-export fallback) and the spike.
- **For a release:** a tag cut now fails at the CI-in-release gate. That is the gate working,
  but nobody on `main` has a green commit newer than 10-23 03:11Z to tag.

## 3. Review of the phone and CI commits

### High

**P1. Android's safe start leaves a real phone with no monitor and no export** (`7df9f8f`, #507;
R from code, not observed). `safe_start` sets `CONCAT_GPU=software` on Android (`lib.rs:2656-2662`).
The Skia patch reads that as "OpenGL ES for the window" (`patches/i-slint-renderer-skia/lib.rs:210`),
which is what was intended. But the compositor reads the same variable as `software_only`
(`concat-render/src/gpu.rs:828-835`), which means `force_fallback_adapter: true` on Vulkan and then
on every backend. On Android the monitor has no window device (`platform.rs:671-685` returns
`None`). Previews draw through `headless()` → `WgpuCompositor::new()`
(`concat-export/src/lib.rs:1643-1666`), and exports through `best_compositor()`. A real phone has
no CPU adapter: SwiftShader is an emulator thing, and the GLES adapter on Mali, Adreno or PowerVR
is not typed Cpu. So both get `None`, previews log "nothing to draw on", exports fail with "this
device's GPU could not be used to draw with", and the cards are left undrawn.

- **It sticks for the whole build.** The GPU switch is hidden on Android
  (`settings.rs:858`, `gpu_switchable: cfg!(any(windows, target_os = "linux"))`).
- **What trips it:** any death in the first 15 s. That includes a swipe from Recents, a Force
  stop, or a low-memory kill while the picker is open.
- **CI cannot see it.** The emulator does have a SwiftShader CPU adapter.
- **It may not fix #507 anyway.** #507 has no logcat (the reporter was asked 10-23), so the
  Vulkan-crash diagnosis is a guess.

Fix: on Android, have safe mode mean `Backends::GL` on hardware for the compositor too (the spike's
`render.rs` already falls back that way), or take Android out of `safe_start` until there is a log.
Show the switch on Android as well.

**P2. Windows trust is now an outage, not a warning** (#533, V). SAC refuses unsigned DLLs once
their reputation lapses. Every DLL beside `concat.exe` (BtbN FFmpeg, sherpa-onnx) is checked on
its own, and SignPath Foundation will not sign upstream binaries (memory, `signpath.md`). Options:

- **Microsoft Store (MSIX).** Registration is free for individuals. The `jub0t.Concat` identity
  is still in repo variables (V). Store packages are signed by Microsoft, which covers every file
  in the package (R; check SAC behaviour on an MSIX install).
- **Azure Artifact (Trusted) Signing.** Paid. It signs any binary you ship, so the FFmpeg DLLs too (U on eligibility).
- **Build FFmpeg in our own CI.** This is #533's option 1.

Also, #533 says turning SAC off cannot be undone. Reports say the April 2026 update KB5083769
made it re-enableable ([Topedia](https://blog-en.topedia.com/2026/04/smart-app-control-in-windows-11-can-now-be-re-enabled-without-reinstalling/),
[Pureinfotech](https://pureinfotech.com/enable-smart-app-control-windows-11/)) (U). Check before
telling users either way.

**P3. Main is red, and the phone signal comes only from quiet hours** (§2). Still H4.

### Medium

**M1. Phone Back skips the phone editor's own sheet.** `app.slint:505-548` closes the desk
sheets, then on Back with "nothing on top" runs `close-project`. The phone shell's sheet (Media,
Text, Effects, Filters, every clip page) is a private `sheet` property of `PhoneEditor`
(`phone/editor.slint:343`) that the handler never sees. So the most common Back on a phone, "put
this panel away", saves and drops you on the launch screen (R). CapCut's Back closes the panel.

**M2. The 0.2.8 notes still carry two false lines** ("Q and E cut", "Intel Macs open on macOS
12 again"). #532 is a 0.2.8 launch death on macOS 13. Edit the body the way 0.2.7's was.

**M3. The dependabot cargo group can never land.** It is one group for every pattern
(`.github/dependabot.yml`), so majors and patches travel together:

- Majors: ureq 2 → 3, cpal 0.16 → 0.18 (Android audio), jni-sys 0.4, pollster 1.0, dirs 7
  (app-data paths), sha2 0.11, getrandom 0.4.
- Patches stuck behind them: rustls, tokio, serde_json, and the sherpa-onnx crate 1.13.8 against
  the 1.13.7 prebuilts in `deps.lock`.

Three PRs in a row have been red on every job. Group by `update-types` (patch and minor
together), and take majors one at a time with a review.

**M4. The Compose spike sits in every CI job.** `concat-mobile` is a workspace member
(`members = ["crates/*"]`). It is a cdylib over concat-host (gpu), export and media, plus uniffi
0.32.2: 24 new lock entries, most of them bindgen-only. So clippy, check and nextest build and
link it on Linux, macOS and Windows. It has 2 tests, and it already turned `main`'s fmt red. It
ships in nothing: the shipped APK is `concat-android` via cargo-apk, and no workflow names
`android-native` or `concat-mobile` (V, grep). Take it out of the workspace or out of
`default-members` until it is a product decision. The cost per job was not measured (no builds
here).

Two smaller points on the spike:
- Its Gradle wrapper has no `distributionSha256Sum`.
- A binary `gradle-wrapper.jar` is committed with no validation step.

**M5. The website goes stale at every release.** Nothing in `release.yml` triggers the website
deploy. Add a `repository_dispatch` on release published, or a daily scheduled deploy.

**M6. The community backlog grows faster than replies.**
- 191 open issues (55 since 20 Oct). 92 of 150 outside issues (61%) have no maintainer reply;
  74 have no comment at all.
- Five crash issues are unanswered: #294 iOS launch (10-09, oldest), #313, #372, #418, #423.
- 7 of 9 contributor PRs have no review. #157 has waited 35 days. #513, #514, #529, #530 and #460
  show only GitGuardian, meaning their CI waits on owner approval.
- Of the 15 newest Discussions, 9 have no reply.

### Low

- **MediaCodec ladder (`2fb84cd`, #491).** Sensible order: as asked, then plain, then
  `c2.android.*` / `OMX.google.*` by `codec_name`. Problems:
  - The plain attempt clamps bitrate to 16 or 32 Mb/s and declares 30 fps for 60 fps input with
    no word to the user. A frame-rate-sized rate control may overshoot.
  - A 10-bit export still tries `c2.android.hevc.encoder`, which is 8-bit only.
  - Tests check only the ladder table, and nothing has run on a device.
- **Mid-export fallback (`0c86f62`).** The structure is sound: one retry, the stalled encoder
  dropped first, the monitor's MediaCodec readers released at export start, and the stage named
  for the sheet and for MCP. It restarts from frame 0, so a long phone export takes twice as long.
  It has never finished a phone CI build.
- **Long-press (`b467155`).** A two-finger pinch that starts on a clip and holds still for 400 ms
  lifts it. There is no haptic at pick-up. The edge of a selected clip still trims on first
  movement, with no slop.
- **Gap closing (`c631c45`).** On a phone the magnet shows as On by default (`prefs.magnetic()`),
  but trims read the raw field (`studio.rs:5110, 5242, 5826`) and do not ripple. Deletes off the
  main lane do not close either. Settings says "deleting and trimming close the gap". One toggle
  shows two states over three behaviours.
- **#525 and #526** ("won't install") were answered well, with the exact size and SHA-256.

## 4. Release and reach (V, `gh release view v0.2.8 --json assets`, 10-24)

| Platform | Assets | Downloads | Share |
|---|---|--:|--:|
| Windows | setup.exe 24,212 + msi 3,030 | 27,242 | 44.5% |
| Android | arm64 apk | 21,377 | 34.9% |
| iOS | ipa | 4,472 | 7.3% |
| Linux | deb 1,410+201, AppImage 1,229+646, rpm 322+106, pkg 254+72 | 4,240 | 6.9% |
| macOS | arm64 3,095, x86_64 770 | 3,865 | 6.3% |
| **Binaries** | | **61,196** | |

- `manifest.json` was fetched 1,667 times and `SHA256SUMS` 1,137 times, so the all-asset total is
  64.0k.
- Updater reach is about 2.7% of binary downloads, against 1% for 0.2.7.
- The repo has 11,181 stars, 1,369 forks and 80 watchers.
- Open issues by label: feature request 90, bug 77, ui-ux 38, windows 30, testing-requested 29,
  crash 25, important 25, timeline 25, video 24, effects 21, android 20, priority 20, audio 18,
  gpu 17, improvement 17, captions 15, AI 14, distribution 13, export 12, needs-info 11,
  editor-api 11, linux 9, ios 8. None are unlabelled.

## 5. Scores (0-10)

| Aspect | 20 Oct | Now | One line of evidence |
|---|:-:|:-:|---|
| CI and release | 5 | **4** | The checks job and 16 KB gate are real, but 4 of 69 `main` CI runs are green, 58 of 65 phone runs were cancelled, `main` has been red about 29 h under four breaks, dependabot cargo has been red 3× and releases still pass unsigned |
| Distribution and trust | 3 | **3** | Site current and vulnerability reporting on; but SAC now blocks installs (#533) on the 44.5% platform, there is no store, no Android registration, and the update manifest is unsigned |
| Documentation (user-facing) | 5 | **5** | The site install guide is honest about SmartScreen/SAC/xattr; the README still ticks iOS ✅ and "4K scrubs smoothly", and the 0.2.8 notes keep two false lines that #532 disproves |
| Design philosophy | 6 | **6** | Long-press, Back to projects, main-lane gap closing and a native Compose spike are platform-first; safe start ports a desktop "software" switch to a platform with no software adapter, and Back ignores the phone's own sheet |
| User likeability on phones | — | **4** | What phones run is 0.2.8: a 4 KB library, Back quits, a tap moves clips; the fixes are 118 commits deep and unreleased, #507 is blank on a Redmi, #294 iOS is unanswered at 15 days |

## 6. Before the next release

1. Get `main` green: fmt, the `clipInspector.*` locale keys, the three clippy lints. Then let one
   full CI run and one phone run finish on the commit to be tagged. Add a local pre-push check
   for fmt and locales.
2. Fix P1: on Android, the compositor uses hardware GLES in safe mode, and the GPU switch is
   shown. Or take Android out of `safe_start`.
3. Fix M1: Back closes the phone editor's sheet before it closes the project.
4. Correct the 0.2.8 notes. In the next notes, name the macOS floor honestly, or make
   `check-min-os.sh --enforce` pass.
5. Windows: pick #533's path and start the Microsoft Store submission (the identity is
   reserved). Say plainly in the README, site and notes that SAC blocks the current build.
6. Wire the website deploy to the release.
7. Register `app.concat.editor` for Android developer verification while the key is stable.
8. Split the dependabot cargo group, and take `concat-mobile` out of the default workspace build.
