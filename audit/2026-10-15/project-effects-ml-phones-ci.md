## project, effects, text, vision, speech, Android, iOS, CI and release: reviewer 4, HEAD ab21457
Baseline 12 Oct at c06560e. Every commit in this part read as a commit; CI, release and issue state via gh. Nothing edited; cargo not run. Metrics script reproduced the 12 Oct table exactly on c06560e (git archive).

### Re-verified (12 Oct numbering)
- 1 Release notes' previous tag: FIXED (31ac1d5). release.yml:174-178 excludes v*-* for a final tag; 0.2.7 notes span 0.2.6 to 0.2.7.
- 2 iPhone Import: UNCHANGED, shipped in 0.2.7. platform.rs:691; only concat-android/src/lib.rs:223 installs a picker; #332; 4 159 .ipa downloads.
- 3 Phones not built on push: UNCHANGED, worse in effect. mobile.yml:53-77 no push trigger; hand runs 76ee423, 7c5a572 red; PR runs for fc7e6a9, a68641c, 7cb4494 died "workflow file issue" (38805162579, 38814642023, 38735268530). No phone build of main since 14b5998.
- 4 Windows blur/glow FXC; GPU suites skip on WARP as passed: UNCHANGED; gpu/tests.rs:41-48. 0.2.7 notes say "a blur or a glow no longer stalls an export" (be1b882, reverted).
- 12 Phone that fails to start vanishes; iPhone plist keys: UNCHANGED. concat-android/src/lib.rs:388-391, 395-397; platform.rs:459-461; src/scripts/ios-app.sh:54-97 lacks UIFileSharingEnabled, LSSupportsOpeningDocumentsInPlace.
- 13 locales.py --check passes with keys missing: UNCHANGED, proven (scripts/locales.py:193-194; vn.json 11 short, red only on parity).
- 14 CI internals in release notes: PARTLY. release-notes.sh:46 filters fix(release|ci); :35/:56 types with digits. 0.2.7 notes still carry the self-contradicting DXC pair, "_exit... rc.2's arm64" (fix(linux)), "BUILT_IN holds 16 locales", "Update Japanese translations". Draft not hand-edited.
- 15 Windows ships CPU ORT, speech asks DirectML: UNCHANGED concat-speech/src/lib.rs:39-41, chatterbox.rs:303-314, concat-vision/src/runtime.rs:145-148.
- 16 Intel Mac ORT: WORSE, confirmed in field (#333, #342; H4). API 27 vs 23 note (build-app.yml:91-96) still for 13.3+.
- 17 --smoke stops before window: UNCHANGED concat/src/lib.rs:1996-1998; scripts claim "through the first frame" (smoke-windows.ps1:17-18, smoke-linux.sh:10-11, smoke-macos.sh:10-11). #339, #365 are this class.
- 18 ISA guard can't catch #193 repeat: UNCHANGED isa-allow.txt:56-63 PROVISIONAL; Ivy Bridge emulator runs --version only (smoke-windows.ps1:177-179).
- 19 Bundle workflow tag-only: UNCHANGED build-app.yml:45-46 workflow_call.
- 20 Signing policy vs workflow: worse in reality: 0.2.7 Windows unsigned (H2).
- 21 Dependabot cargo group red: UNCHANGED dependabot.yml:27-36; #327 holds 18 updates.
- 28 Concurrency cancels main: UNCHANGED ci.yml:31-32; six runs 16 Oct 05:55-06:01 cancelled.
- 29 ubuntu-latest -> Ubuntu 26 on 19 Oct: UNCHANGED, 4 days. ci.yml:37, mobile.yml:96, 262, release.yml:42, 118, nix.yml:35, models.yml:41.
- §7 smoke-ios exit status: FIXED smoke-ios.sh:193. smoke-android trap: FIXED :118. exec bit: FIXED (100755). Simulator env inline: UNCHANGED mobile.yml:485-496; mobile-env.sh:17 only 2 device targets. "Record emulator first result as baseline": WORSE (baseline is "never runs app code", H3).
- Phone downloads no digest: UNCHANGED ffmpeg-mobile.sh:38-39; sherpa-mobile.sh:38-39, 56-57.
- Cards skip with no adapter: UNCHANGED concat-host/src/cards.rs:602.
- project AddClip whole media: UNCHANGED commands/clips.rs:34-38. Speed curve grows over neighbour: UNCHANGED commands/properties.rs:285-305 (SetClipSpeed ripples :108-118, curve doesn't). DOCUMENT_VERSION 1 fine (doc.rs:41; newer refused doc.rs:142, test lib.rs:1691). All tests one file: WORSE lib.rs 4 294 -> 4 533, 121 tests.
- effects never through D3D12: UNCHANGED; Linux coverage now gone too (H1).
- android three bridge crash paths, 0 tests: UNCHANGED; emulator never reaches it; new paths M1, H5.

### Commits
- 31ac1d5 notes from previous release: works. Misses CI-internal subjects with product scope (fix(linux), fix(windows)) and the be1b882/3178b78 contradiction; not hand-edited.
- 76ee423 phone builds started on emulator/simulator: iOS genuinely good (v0.2.7 "smoke: ok", exit checked, app up 60 s, first log line). Simulator build is separate; shipped .ipa never launched; newest runtime only. Android never ran app code (H3); mobile.yml:33 claim false for Android.
- 7c5a572 / 0660c01 emulator crash "not tested": diagnosis plausible (ORT cpuinfo_arm_linux_init under ndk_translation); not hiding real-phone crash (untestable_here returns early unless translating, smoke-android.sh:242). But turns gate into permanent no-op reporting success; API 30 blind matcher (:262-266) accepts any SIGSEGV at 0x8 before first line. "Started on a real phone by hand" has no record.
- b483314 0.2.7 without Windows ARM: honest; #326 open cause unknown; arm64 Windows cell blank, nothing says x64 runs emulated. Release run cancelled and v0.2.7 tag MOVED to 14b5998 (runs 38430320304, 38431729500).
- e525d7c thin LTO Intel Mac: landed 18:04 after v0.2.7 build (16:35); 0.2.7 Intel is fat LTO; unproven; optimises a bundle that doesn't start on its declared minimum.
- 447570a encoder retry: patient final drain as asked; never against MediaCodec in CI; #286 unconfirmed.
- b607491 exit 1: CI-only benefit.
- c71f900 speech stages: clean (JobStage, transcribe.rs:423-436, tts.rs:884-903); zero-window safe; one test; tts::speak 313 lines.
- fc7e6a9 ConsolidateClips: best-validated command (clips.rs:893-1004: grid, edge transitions, content underneath, detached audio, full selection); 7 pure tests incl. undo. Two non-test expects (clips.rs:634 safe, :976). Merged with failing PR CI (38805162274); host test consolidation_preserves_the_timeline_clock_of_animated_effects hits 300 s TIMEOUT on WARP (run 38815298005), lacks heavy_gpu guard. Orphans (L4).
- a68641c: window/locales only; merged on red; touches ui/phone/editor.slint with no phone build.
- 5add799 (#364): every effect name English in 24 locales; deliberate.

### Release and CI state
- v0.2.7 published 13 Oct 18:23 from 14b5998 after tag moved; 13 bundles. Downloads: Windows setup 17 077 + msi 1 838; Android apk 16 001; iOS ipa 4 159; macOS arm64 2 701; macOS x86_64 640; Linux ~2 580. First release with every desktop launch check passing (Linux arm64 _exit fixed; Windows x86_64 through emulated Ivy Bridge). Windows UNSIGNED ("No signing certificate: the build is unsigned.", log 17:32:45). macOS ad-hoc, not notarised. Android signed with real key (V3, CN=Concat, SHA-256 923308...6e2e). Android smoke API 30 and 35 "NOT TESTED", exit 0.
- main red since 14 Oct 05:05 (f064b68, #304 vn.json). 7 failed + 7 cancelled since; last green e525d7c. Engine: locales --check fails at ci.yml:141, before clippy (:150) and tests (:168). Windows: i18n test (concat/src/i18n.rs:447) + fc7e6a9 test timeout. Six PRs merged on red incl. two contributor features.
- Pipeline weaknesses: tag-only (19), phones not on push (3), Android gate no-op (H3), signing skips silently (H2), no check bundled libraries' minimum OS vs declared (H4).
- Supply chain: every action SHA-pinned; desktop downloads digest-pinned; phone downloads not; ci.yml, mobile.yml, build-app.yml no permissions: block (L2).
- 106 open issues (82 on 12 Oct); 6 open PRs (19).

### Open issues that matter
Windows start-up: #339 closes ~5 s after start (undiagnosed, no log asked); #365 main access violation on hybrid NVIDIA/AMD right after "concat cards" thread starts (suspected second device/driver; CI can't see: WARP + --smoke); #306, #311 Smart App Control (unsigned keeps open); #326 Windows ARM crashes on --version; #313, #318 AVX2 (probably fixed, unconfirmed, open); #297 FXC stalls; #291 DirectML.
Intel Mac: #333 open, #342 closed by reporter, no fix.
Android: #312 Android 11 preview (fix shipped, unconfirmed); #286 export codecs (unconfirmed); #353, #328 safe area; #351, #354, #355, #356 layout clipping; #229, #221, #151.
iOS: #332 Import dead; #338 how to install; #294 crash at launch iOS 16.6; #299 fix shipped, unconfirmed.

### New findings
HIGH
- H1 main no Linux tests, clippy or effect-package GPU run for two days; features merged anyway. ci.yml:141 aborts before :150/:168; lavapipe is the only CI place the packages render (Windows skips gpu/tests.rs:41-48, macOS compiles). Eleven commits untested on Linux incl. fc7e6a9, a68641c. Windows also fails (i18n test, consolidation timeout). Fix vn (vi exists); stop merging on red.
- H2 0.2.7 Windows shipped unsigned; nothing stops it. build-app.yml:946-949 exits 0 without WINDOWS_CERTIFICATE_PFX, on a tag too (verified in log). SAC refuses unsigned DLLs (#306, #311); SmartScreen warns everyone. Fail tag build without cert, or say "unsigned" in notes.
- H3 Android launch check never tested the app and gates release as passed. Emulators x86_64 (mobile.yml:295-309), APK arm64-only; every Android smoke (rc.3, rc.4, v0.2.7) ended untestable_here exit 0 (smoke-android.sh:241-277). 16 001 APK downloads never started by CI; #312, #286 are what it was added for. Build x86_64 APK from same source for emulator, or device farm.
- H4 Intel Mac 0.2.7 doesn't start on macOS 12.0-13.2. dyld Symbol not found __ZNSt3__18to_charsEPcS0_d from libonnxruntime.1.23.2.dylib (libc++ symbol from 13.3). Declares 12.0 (build-app.yml:102, LSMinimumSystemVersion :582), links runtime at load (:294-302, ORT_PREFER_DYNAMIC_LINK=1). Launch check macos-15-intel only. 2015-2017 Macs on Monterey crash (#333, #342). On 13.3+ cutouts/Chatterbox face API 27 vs 23. Load ORT lazily (load-dynamic) or raise minimum to 13.3; add otool -l minimum-OS check over every dylib.
- H5 (suspected, from android-activity 0.6.1 and Slint) Android closes whenever activity recreated. concat-android/Cargo.toml:66 only orientation|keyboardHidden|screenSize. Dark mode (incl. scheduled), split screen, display size, font scale, locale recreate NativeActivity; android-activity calls android_main again (its lib.rs:66-79). Second call: set_var with worker threads alive (lib.rs:31-39, SAFETY false), slint::android::init fails (platform set), return (:388-391) finishes activity. App vanishes mid-edit. Add uiMode|screenLayout|smallestScreenSize|density|fontScale|locale|layoutDirection; guard second android_main.
MEDIUM
- M1 (suspected) Android import crashes after process kill or provider error. ConcatFiles Fragment via in-memory dex (lib.rs:154-195), added for pick (ConcatFiles.java:146-150); restore after kill -> ClassNotFoundException, pick (in-memory PENDING) lost. copy() catches only IOException (:262-289); SecurityException from openInputStream (revoked grant, cloud URI) kills app.
- M2 Android projects/imports wiped on uninstall, unreachable. HOME = external_data_path (lib.rs:37-38); every pick copied to getExternalFilesDir/Imported (ConcatFiles.java:203-204). Android 11+ hides Android/data; "folder the user can reach" false. Reinstall deletes everything; imports double storage.
- M3 Release notes told ~19 000 Windows users a false thing (14 above). Published, never corrected.
- M4 Shipped iOS app never launched. Simulator smoke newest runtime only (smoke-ios.sh:131-148); MinimumOSVersion 15.0 (ios-app.sh:21, 67). Nothing checks prebuilt SherpaOnnxC/FFmpeg minimums vs 15.0 (H4 class). #294 open.
- M5 Nobody knows if main builds for phones today; a68641c changed phone editor without a phone build.
- M6 (suspected) Cutouts no CPU fallback on accelerator run-time failure (concat-vision/src/runtime.rs:62-80, 115-121), unlike speech's #291 retry. NNAPI (deprecated Android 15) and CoreML never in CI.
- M7 No offline shader portability check: naga only wgsl-in (concat-effects/Cargo.toml:15); packages reach D3D12 and GLES first on users' machines. Pure naga hlsl-out/glsl-out test over every package, no GPU needed.
LOW
- L1 concat.light-leak-flash/effect.wgsl:17 pow(leak_pos - progress, 2.0) negative base, undefined in WGSL/HLSL/MSL; NaN band on a driver that doesn't fold it. Write d*d. (suspected)
- L2 No top-level permissions: in ci.yml, mobile.yml, build-app.yml.
- L3 v0.2.7 tag re-pointed after cancelled run, tagged while 14b5998's CI running.
- L4 Consolidation orphans renders (concat/src/studio/consolidate.rs:101-112); HDR merge 10-bit HEVC hardware-first -> 12 Oct NVENC crash reachable from a right-click.
- L5 concat-text/src/lib.rs:569 mmaps fonts; replaced/truncated font = SIGBUS. Known.
- L6 English word-for-word lines 993 -> 1 357 (CI log); effect names now always English (5add799).

### Numbers
project 10 685 (+400) tests 121 (1) (+7) unsafe 0 unwrap 376 (13) (+2) panic 6
effects 7 256 (0) ~1 785 WGSL, 123 packages (125 after #361) tests 72 (9) unsafe 0 unwrap 134 (5) panic 2
text 2 607 tests 24 (1) unsafe 1 unwrap 67 (1)
vision 2 760 tests 31 (8) unsafe 0 unwrap 22 (2)
speech 3 411 (+104) tests 22 (3) (+1) unsafe 3 unwrap 23 (2)
android 398 + 291 Java tests 0 unsafe 10 unwrap 1 (1)
Biggest fns: commands/clips.rs:12 apply 670; commands/properties.rs:12 apply 338; tts.rs:719 speak 313; concat-text/src/lib.rs:1201 paint 302; catalogue.rs:193 from_manifest 220. project lib.rs 4 533 lines, all 121 tests.
CI: workflows 2 396 lines (mobile.yml +201, build-app.yml +11, release.yml +8); scripts/ci 1 350 (+576: two phone smokes, Show-Crash smoke-windows.ps1:65-84). Model downloads fail closed on missing/wrong digest (concat-host/src/models.rs:295-310).

### Scores (CQ Maint Scale Perf Arch Phil Sec Test)
project 8 6 6 6 8 7 6 8
effects 8 7 6 5 8 8 6 6↓
text 7 6 5 6 8 8 7 6
vision 8 8 5 5 9 8 8 7
speech 7 5 6 6 7 7 8 6
android 6↓ 7 – 7 6↓ 7 7 1
CI and release: 4↓ (was 6)
Features: Effects/filters 8→7↓; Templates 7→7; LUT 8→8; Custom/community packages 7→7; Cutout/brush 6→5↓; Enhance 5→5; Captions engine 7→7; TTS 7→7; Voiceover 6→6; Export on phone 6→6; ★Phone launch checks Android 2, iOS 7; Bundle launch checks 6→6.

### Good
iOS simulator smoke is real (exit checked, 60 s watch, first log line), passed on v0.2.7. 31ac1d5 works. ConsolidateClips refuses rather than loses, rechecks before adopting (concat-host/src/consolidate.rs:51-68), one undo step, tests prove each refusal. Model downloads fail closed (models.rs:295-300). Effect packages can't reach outside: FFmpeg filter allowlist (filters.rs:22-190), LUT plain .cube beside manifest (manifest.rs:534-537), reserved built-in author. Show-Crash records faulting module from Windows event log (smoke-windows.ps1:65-84). Android release signing correct, verified on shipped APK.
