## Window crate (src/crates/concat): reviewer 1, HEAD ab21457

Counting reproduced 12 Oct exactly on c06560e (23 923 lines, 37 (4) unwraps, 13 unsafe, 87 tests). Did not run cargo; ran locales.py --check; read CI via gh.

### Re-verified (12 Oct numbering)
- 2 iPhone Import does nothing: UNCHANGED. platform.rs:675-694 only log::warn (:691). install_file_picker only at concat-android/src/lib.rs:223; install_export_publisher :321; nothing on iOS.
- 5 >24 titles drop out mid-drag: UNCHANGED. studio.rs:2733-2749 hold_still per title -> preview.rs:169 -> concat-media/src/pool.rs:541 (STILLS_HELD=24).
- 6 Shared save scratch: UNCHANGED. concat-host/src/projects.rs:174; saves from studio.rs:2633 and close_project :7300-7301.
- 8 Captions sheet transcribes a clip it isn't showing: UNCHANGED, one site worse. select.slint:127, segmented-control.slint:43, inspector/controls.slint:559 still self-assign; captions.slint:118,132,143,216,225 as were; 34 one-way bindings into these (new: settings.slint:932).
- 9 studio.rs growth: WORSE. 11 161 (+295). First split landed: studio/consolidate.rs 213.
- 12 Phone that can't start vanishes; iPhone files invisible: PARTLY. b607491 logs + exit 1 (lib.rs:2027-2028); report_startup_failure still no-op on phones (platform.rs:445-461); ios-app.sh lacks UIFileSharingEnabled / LSSupportsOpeningDocumentsInPlace.
- 13 locales.py --check passes with missing keys: UNCHANGED. scripts/locales.py:193-194 never sets failed.
- 17 --smoke stops before window exists: UNCHANGED lib.rs:1996-1999.
- 18 Window smalls: UNCHANGED host.rs:360-366, dock.rs:19-20, meters-pane.slint:198,206,214.
- 22 4px drag threshold < finger slop: UNCHANGED timeline/lanes.slint:591-592.
- 23 #151 after Browse: UNCHANGED panes/start.rs:165,175,215,243.
- 24 Detach unmutes deliberately muted video: UNCHANGED studio.rs:10522-10523.
- 25 Unknown argument refuses to start: UNCHANGED launch.rs:42-47.
- 26 Commits that close issues: WORSE. e042957 "Fixes #335", 3479407 "Closes #320" closed both; neither in a release (0.2.7 shipped at 14b5998).
- 27 RTL laid out LTR: UNCHANGED.
- 33 Doc drift: UNCHANGED studio.rs:2259-2265, :7632, :10335, :1026 (doc for absent label_of atop shelves). concat/src/gpu.rs:62-66, 96-100 .ok()? drops adapter/device errors unlogged.
- 11th#3 Paste keeps shape: PARTLY FIXED (e042957). Footage copies right (copy_commands studio.rs:1948-2027, tested). Titles/shapes carry only style, duration, offset_y (:6993-7017): scale, x, rotation, opacity, keys, fades, effects, blend lost. cutout, audio_stream, enhanced never copied. close_project (:7286) leaves clipboard full.
- 11th Literals through I18n: UNCHANGED + new. studio.rs:3695, 6784, 7278-7279, 9679 ("Freeze frame", "F" shortcut skipping platform::keys), :4815 "Title", :1021 "New title", :1047 "Other"; Merge refusal reasons (M5).
- 12th platform.rs:347-350 SAFETY untrue: UNCHANGED + new L1.
Tally: 0 fixed, 2 partly, 13 unchanged, 3 worse.

### Commits since 12 Oct
- b607491 exit 1 when event loop can't run: does it; phone user still sees nothing; no test.
- 14b5998 own device on Vulkan (#324): off_thread() for export (panes/export.rs:518), cards (studio.rs:7481), merge (studio/consolidate.rs:116). On Vulkan incl. Android (gpu.rs:51-57) every export and every batch of cards opens a fresh wgpu device (cards.rs:404-407). Never run on a phone. No test.
- 8f5b141 crash handler: window side fine (call_once).
- c71f900 staged progress: panes/pace.rs pure, 5 tests. Progress redraws every frame while live (L3). No run id on progress messages (L8 suspected). All 25 locales translated by maintainer.
- 3479407 + 5c698aa interface scale (#320, contributor): 4 pure tests, runtime untested. Scale absolute not relative (M1). False SAFETY on runtime set_var (L1). 3 keys English in 24 locales. Phone gate right. Never built for phones (H2).
- e042957 copy of split clip (#335): correct, tested at speed 1 and 2. Duplicate still two undo steps (M4). Used "Fixes".
- fc7e6a9 consolidate selected clips (contributor): staleness recheck before adopting (concat-host/src/consolidate.rs:52-66), cancel, Drop guard cleanup, window + host export test. REMOVES lossless MergeClips path (M5). Progress only in tray label. Refusals English. 10 keys English in all locales. merge_blocked in publish_lanes (L2). Not on phones. No issue ref.
- a68641c caption style to all titles on track: one undoable batch, tested undo/redo. Silent when nothing changes; strips receiving titles' position keys (L4). English in 23 locales.
- 5add799 effect names out of locales (#364): does what it says; en.json 673 keys. Edited vn.json without noticing it isn't compiled in.
- 5a623c0/013d284 contributor vn.json, merged via f064b68 (PR #304): BROKE main (H1).
- bdcaedf ClearType: sound design (LCD text only on opaque swapchain); never looked at on a Windows screen (L7).

### New findings
HIGH
- H1 main red since 14 Oct; Linux clippy+tests not run since. vn.json stale second Vietnamese locale, not in BUILT_IN (i18n.rs:57-83), 11 keys behind. locales.py --check exits 1 on parity (:157, :229). i18n::tests::every_locale_file_ships_and_each_once fails (CI Windows run 38815298005). On Engine job the failed locale step skips clippy, tests, doc tests and gRPC on Linux. Every completed push since f064b68 failed/cancelled; #329, #336, #357, #359 and 5add799 landed on red tree. Fix: delete vn.json or fold into vi.json. (verified by running)
- H2 No window change since v0.2.7 built for Android or iOS. mobile.yml fails as "workflow file issue" on every contributor PR (3479407, e042957, fc7e6a9, a68641c); last two hand-started runs on main failed (76ee423, 7c5a572). (verified via gh)
MEDIUM
- M1 Interface scale replaces display scale instead of multiplying. lib.rs:169-183, 1486-1530; prefs.rs:15-18, 141-148; hint en.json:416. Slint winit treats SLINT_SCALE_FACTOR as absolute (i-slint-backend-winit-1.18.1/winitwindowadapter.rs:602-608, 666-668). Windows laptop at 150%: "100%"/"125%" makes UI smaller; Retina "100%" halves it. While set, moving to another monitor no longer rescales (:1544). Fix: multiply by winit scale_factor, re-apply on scale event. (verified against Slint source)
- M2 Export sheet can show a codec it won't use. dialogs/export.slint:392-403 (no disabled list), select.slint:125-128 assigns current, panes/export.rs:171-176 refuses codec device can't encode. Pick AV1 without AV1 encoder (most Android, many Windows): picker says AV1, file is H.264. Since 8d3bf19 (28 Sep).
- M3 Phones: dialogs and toasts ignore safe area. Only main layout inset (app.slint:594-597). Modals at window root full size (app.slint:677-906), surface up to height-24px (modal.slint:48-57). ToastLayer 16px above bottom (toast.slint:86-88). Edge-to-edge Android (targetSdk 35)/iPhone: toast under nav bar/home indicator, sheet header/close under status bar. (code verified; device effect reasoned)
- M4 Duplicate is two undo steps; first leaves whole source file on timeline. studio.rs:6885-6890 place_copy(..., None); place_copy :6986-7040 applies AddClip (whole file at lane end) then shaping Batch separately (concat-project/src/editor.rs:187-205). Ctrl+Z after duplicating 3s of a 2h file leaves a 2h clip. Fix: pass a gesture like Paste.
- M5 Merge re-encodes where it used to rejoin losslessly. fc7e6a9 dropped MergeClips path. merge() (studio/consolidate.rs:71-200) renders to media/merged-*/merged.mp4, H.264 crf16 fast or HEVC for HDR (concat-host/src/consolidate.rs:118-131). Refused for audio/image clips, clips above other visible content, edge transitions, off-grid cuts (concat-project/src/commands/clips.rs:894-1000). Progress only in tray label, no tooltip (timeline/tray.slint:277-288). Refusals English (consolidate.rs:49-52 error.to_string(); host :34, :62).
LOW
- L1 runtime set_var with false SAFETY: lib.rs:1498-1507 while decoder/audio/scheduler/export threads run; Linux getenv race in FFmpeg/libva/Mesa. "System" removes a hand-set value against :166-167. +2 unsafe (:178, :1499).
- L2 merge checks every lanes publish: merge_blocked() in publish_lanes (studio.rs:8010-8015) every playback tick/drag move; consolidation_clips clones every selected clip (clips.rs:905-912). selection.iter().any per visible clip (studio.rs:7843) O(visible x selected).
- L3 progress bar redraws every frame while live (primitives/modal.slint:333-338, 373); on phone competes with whisper. (reasoned)
- L4 "Apply to all" silent and destructive: no feedback when nothing changes (studio.rs:5000-5017), strips OffsetX/OffsetY keys (:1373-1379), button shows when title alone on lane (inspector/text-panel.slint:172-188).
- L5 contributor PRs add English to every locale: 3479407 (3), fc7e6a9 (10), a68641c (2). 811 values still English across 24 locales.
- L6 vn.json edited three times while never compiled in.
- L7 ClearType unseen on Windows; three SystemParametersInfoW calls per frame.
- L8 captions cancel/restart collide (suspected): no run id (panes/captions.rs:161-170); late Finished(Err cancelled) closes reopened sheet.
- L9 merge interrupted by quit leaves media/merged-* (Linux _exit lib.rs:2046-2055 skips Drop guard).

### Numbers
Rust 24 885 (+962). studio.rs 11 161 (+295, 45%); impl Studio :2093-10497 8 405 lines, 219 methods, 94 fields. start() lib.rs:162-2031 1 870 lines (+98). Next: publish_lanes 389, publish_chrome 280, stage_dragged 262, key_editor_rows 255, settings update 247, menu_bar 224. unwrap/expect 59, 4 non-test (host.rs:171, panes/start.rs:152, studio.rs:2711, :9006); panic 0. unsafe 15 (+2). Tests 96 in 22 files (+9). Slint 30 081 (+180); 34 one-way bindings. Locales 26 files (25 + stray vn), 673 keys, placeholders match, --check exits 1.

### Scores
concat (Rust): CQ6 Maint4 Scale5 Perf6 Arch7 Phil6 Sec6 Test5
concat (Slint): CQ7 Maint7 Scale6 Perf6↓ Arch6 Phil6
locales: CQ6↓ Maint8 Scale7 Arch8 Phil8 Test7
Features: Launcher 8→8; Import 8→8; Timeline editing 8→7↓; Copy/paste 6→7↑; Undo 8→8; Titles 8→8; Auto-captions 7→7; TTS 7→7; Export 8→7↓; Export on phone 6→6; Phone shell 6→5↓; Languages 8→7↓; Themes 8→8; ★Interface scale 4; ★Merge selected clips 6; ★Caption style across track 7; Windows text (ClearType) 5→5; Command-line launch 7→7; Accessibility 2→2.

### Good
pace.rs pure with 5 tests; studio/consolidate.rs first feature out of studio.rs, Drop guard, adopt only if inputs unchanged, epoch-tied; e042957 precise with failing-without-fix test; a68641c test covers undo/redo/other lane/keys/missing id; 5add799 locales smaller and honest; the gate caught vn.json, nobody acted; non-test unwrap 4, panic 0 in 25k lines.
