## Window crate (src/crates/concat), HEAD ce64dae, 2026-10-20

Method: I read the code at HEAD and `git log`/`git show` since ab21457 (15 Oct). I ran `scripts/locales.py --check` (python only). I did not build, run cargo or open GitHub, so CI and on-device behaviour are **unverified** throughout.

**Coverage gap, stated plainly.** Four parallel deep-reviews did not report back before hand-off. They covered: today's commits (be60358, f121747, 64e9dcd, a1fd411, f2d0637, ce64dae, b18c2fe/a9b2df6); keymap/Shortcuts; the Audio page with its CC0 download shelf; Voiceover and batch volume; phone gestures (glide.rs, pinch, hold-to-repeat, Mask, Canvas and Adjustments sheets, Android back); the project lock (68ceffe); `--mcp` and the Automation token; and whether fa651ef's scale fix is correct against the Slint winit source. Those areas are **not audited** here except where a line below says I checked something myself. The rest of the crate changed by 99 commits and +21.9k/-4.6k lines in 5 days, so this pass is necessarily partial.

### Numbers (measured)
| | 15 Oct | 20 Oct | Δ |
|---|---|---|---|
| Rust lines (src/) | 24 885 | **33 163** | +8 278 (+33 %) |
| Slint lines (ui/) | 30 081 | **34 913** | +4 832 |
| studio.rs | 11 161 | **12 739** | +1 578 (38 % of the Rust) |
| `impl Studio` | 8 405 lines, 219 methods, 94 fields | **9 854 lines (2251-12104), 247 methods, 106 fields** | worse |
| `fn start` (lib.rs:196-2435) | 1 870 | **2 240** | +370 |
| Largest fns | publish_lanes 389 | publish_lanes 396, publish_chrome 353, key_editor_rows 330, stage_dragged 288, clip_set 264, menu_bar 242, settings `update` 285 | |
| unwrap/expect (all / non-test) | 59 / 4 | **112 / 4** | non-test: host.rs:174, studio.rs:2932, studio.rs:10219, panes/start.rs:230; all four are sound invariants |
| `unsafe` lines | 15 | **34** | +19, of which 20 are in the new ios.rs (objc msg_send) and 9 in wayland_drop.rs. Runtime set_var sites: lib.rs:2472, platform.rs:427 |
| `#[test]` | 96 in 22 files | **155 in 28 files** | +59 |
| Locales | 26 files (+stray vn), 673 keys, --check exit 1 | **25 files, 881 keys, --check exit 0**, 707 values still English | |
| Studio splits | consolidate.rs | + backdrop.rs 306, masking.rs 409 | |

### Re-verified findings from 15 Oct
| # | Finding | Status at HEAD | Evidence |
|---|---|---|---|
| 2 | iPhone Import does nothing | **FIXED in code** (device unverified) | lib.rs:200-201 `ios::install()`; ios.rs:100-133 installs the picker and publisher |
| 5 | >24 titles drop out mid-drag | UNCHANGED | studio.rs:2953-2970 hold_still per title; concat-media/src/pool.rs:541 STILLS_HELD=24 |
| 6 | Shared save scratch | **FIXED** | concat-host/src/projects.rs:300-330 per-call `create_new` temp (pid + nonce). The doc at :267-271 still says "a project folder has none [lock] today", which is stale since 68ceffe (Low drift) |
| 8 | Captions sheet self-assigning bindings | UNCHANGED | select.slint:140, segmented-control.slint:47 assign `root.current`; captions.slint:193, 204, 226, 235 bind one-way |
| 9 | studio.rs growth | **WORSE** | 12 739 lines (+1 578); two small splits do not offset it |
| 12 | A phone that can't start vanishes; iPhone files hidden | PARTLY | Files: plist keys added (src/scripts/ios-app.sh:99-103). Phone startup failure is still a no-op (platform.rs:536-537). Android gets no safe start (lib.rs:2455-2457) |
| 13 | locales.py passes with missing keys | UNCHANGED | scripts/locales.py:193-194 prints "missing" but never sets `failed` |
| 17 | `--smoke` stops before the window exists | UNCHANGED | lib.rs:2366-2385 exits before `app.run()` (:2417), so the swapchain and first frame (#389-class crashes, 369fdfa) are never exercised |
| 18 | Window smalls | PARTLY | meters-pane.slint and METERS_MIN_W are gone with the VU rewrite. host.rs:102, 281, 359, 386, 406 still drop `invoke_from_event_loop` errors |
| 22 | 4 px drag threshold below finger slop | UNCHANGED | timeline/lanes.slint:591-592. The new finger gesture uses 6 px (:1204-1206), still under Android's 8 dp |
| 23 | #151 returns after Browse | UNCHANGED | panes/start.rs:255 and :295 set the location without recomputing the free "Untitled" name (only Compose at :243 does) |
| 24 | Detach unmutes deliberately muted video | UNCHANGED | studio.rs:12124-12134 (`Again` for any muted video), :11953-11964 |
| 25 | Unknown argument refuses to start | UNCHANGED | launch.rs:73-78. Windows "Open with" and Linux `%U` still produce an error box |
| 26 | Commits that close issues | **FIXED this period** | none of the 179 commits since 15 Oct uses a closing keyword (word or URL form). PR bodies of the merged #377/#401/#402/#403 are unverified |
| 27 | RTL laid out LTR | UNCHANGED | no layout-direction handling anywhere in ui/ or src/ |
| 33 | Doc drift; gpu.rs drops errors | UNCHANGED + new | gpu.rs:87, 122 `.ok()?` drops adapter and device errors unlogged. New drift: launch.rs:18 says "Concat (software drawing)" but the installer says "Concat (safe graphics)"; lib.rs:2469-2470 SAFETY cites "the scale above", which no longer sets env; studio.rs:10212-10215 has two stacked doc comments |
| 11th#3 | Paste/duplicate keeps shape | **WORSE** | Titles and shapes still carry only style, duration, offset_y, name (studio.rs:8007-8031). `copy_commands` (studio.rs:2106-2180) now also misses the **new** `mask`, `stabilize` and `background`, plus `cutout`, `audio_stream`, `enhanced`. Clipboard survives close (see N1) |
| 11th | Literals bypass i18n | UNCHANGED + new | studio.rs:1140 "New title", :1225 "Other", :5352 "Title", :11204 "Freeze frame", :4097, :7820, :8328-8329 English notices; consolidate.rs:52, :107 pass `error.to_string()` |
| 12th | platform.rs SAFETY "single-threaded" untrue | UNCHANGED | platform.rs:424-427 sets SLINT_WGPU_CPU *after* `Gpu::acquire()` (:409) has created a wgpu device. On lavapipe that device has started threads, so "single-threaded" is false |
| H1 | main red; stray vn.json | **FIXED locally** | vn.json deleted (d3f612e); `locales.py --check` exit 0. CI state unverified |
| H2 | Phones never built | CONFIG FIXED, runs unverified | .github/workflows/mobile.yml:77-84 now builds on push to main (5195bb2) |
| M1 | Interface scale replaces display scale | **FIXED per code** (correctness vs Slint internals unverified) | platform.rs:44-102 multiplies winit's `scale_factor()` and re-applies after a monitor change; SLINT_SCALE_FACTOR set_var removed |
| M2 | Export picker shows a codec it won't use | PARTLY | Details now read "not on this device" (dialogs/export.slint:329-332). But the Select still gets no `disabled` list, so select.slint:140 assigns AV1, panes/export.rs:149-153 refuses it, and the picker keeps showing AV1 while the export writes H.264. One-line fix: pass `disabled` |
| M3 | Phone dialogs/toasts ignore safe area | PARTLY | Every sheet and floating panel now takes the insets (app.slint:642-645, 683-686 … 927-928; modal.slint:46-99). **The toast does not**: app.slint:975-981, toast.slint:86-88 put it 16 px off the window bottom, i.e. under the Android nav bar or iPhone home indicator |
| M4 | Duplicate is two undo steps | UNCHANGED | studio.rs:7924 `place_copy(..., None)`; AddClip of the whole file (:8063-8071), then a separate Batch (:8072-8073) |
| M5 | Merge re-encodes instead of a lossless join | UNCHANGED | studio/consolidate.rs untouched since; the window never issues `MergeClips` (concat-project commands/mod.rs:623) |
| L1 | Runtime set_var with false SAFETY | PARTLY | Scale set_var removed. safe_start's set_var (lib.rs:2472) is pre-thread and plausibly true; platform.rs:427 is not (above) |
| L2 | merge_blocked in every lanes publish | UNCHANGED | studio.rs:9141 inside publish_lanes (8897-9292) |
| L3 | Progress bar redraws every frame | UNCHANGED | primitives/modal.slint:416, 455 `animation-tick()` |
| L4 | "Apply to all" silent and destructive | MOOT | feature removed in 13f6d4f (-286 lines) a day after it merged |
| L5 | Contributor strings English in every locale | PARTLY | merge (10 keys) and interface scale (3 keys) still English in 21-22 of 24 locales (`--check --same`); id and pt-BR translated |
| L6 | vn.json edited while never compiled in | FIXED | deleted |
| L7 | ClearType never seen on Windows | **MATERIALISED, then fixed** | 369fdfa: bdcaedf crashed Windows on the first frame of text (MSVC by-value return ABI in `top_props`). Fixed before v0.2.8. Code lives in src/patches/i-slint-renderer-skia (outside this crate). SystemParametersInfoW still runs per frame (dx12.rs:75-78) |
| L8 | Captions cancel/restart collide | UNCHANGED, worse than suspected (reasoned) | panes/captions.rs:184-188 Cancel clears `running`, so Begin can start a second run. The first run's late `Finished(Err cancel)` then runs `self.subject.take()` (:192), stealing the **second** run's subject. The second run's Ok then returns at :199-201 and its captions are dropped silently; :229 also closes the sheet. Cancellation is detected by `error.contains("cancel")` on English text |
| L9 | Merge interrupted by quit leaves media/merged-* | UNCHANGED | lib.rs:2511-2524 `_exit`/`process::exit` skip Drop guards |

Tally: 6 fixed, 8 partly, 15 unchanged, 3 worse, 1 moot.

### New findings (verified unless marked)
**MEDIUM**
- **N1 Paste after switching projects inserts the wrong footage.** `release_project` (studio.rs:8370-8432) never clears `self.clipboard` (field :860, only set at :7959). Ids are per-project counters (`IdMint`, concat-project/src/commands/mod.rs:890-900: "m1", "m2" …), so a copied clip's `media_id` names a *different* file in the next project. Scenario: copy a clip of `m3` (beach.mp4) in project A, close, open project B, Ctrl+V. `place_copy` (studio.rs:8063-8071) adds B's `m3` (say interview.mov) with A's trims, keys and effects. If B has no `m3`, the paste fails with nothing said. Paste stays enabled (:11169). Fix: clear the clipboard in `release_project`, or key it by project.
- **N2 Copy/Paste/Duplicate drops the features added this week.** `copy_commands` (studio.rs:2161-2179) builds its `ClipPatch` without `mask` (16a3d13, #239), `background` (Canvas, 244765e) or `SetClipStabilize` (915babd), and also skips `cutout`/`audio_stream`/`enhanced`. Model fields: concat-project model.rs `Clip { … cutout, mask, stabilize, … background }`. Scenario: mask a clip with a heart, Duplicate: the copy is unmasked and unstabilized. The only test (studio.rs:12266) covers split frames. Add a field-exhaustiveness test (destructure `Clip` without `..`).
- **N3 One early death of any cause puts the whole app on the CPU rasteriser for good.** safe_start (lib.rs:2454-2475) flips `software_gpu=true` permanently when the start mark is left. Things that leave it: any death before 15 s (SAFE_AFTER, :2437), including Task Manager, a logoff or shutdown, a panic opening a corrupt `--project`, or a blur stall killed by hand. From then on `CONCAT_GPU=software` is set every launch, and the shared device is WARP/lavapipe for preview, effects *and export* (gpu.rs:76-87; Host::start(gpu) lib.rs:256). You get one toast (lib.rs:282-284) and a switch in Settings. Nothing re-tries the GPU, even after a driver update. Also: `--smoke` exits via `leave()` (lib.rs:2381, :2385) without dropping `startup_mark`, so any smoke run on a user machine forces the next real launch into software. Design trade-off defensible for #339/#389, but it should be one-shot (retry the GPU after N clean software launches, or ask).
- **N4 Shortcut, scale, GPU and every other preference can vanish on a crash.** prefs.rs:267-271 `fs::write` straight over the file (not atomic). prefs.rs:258-262 then treats any parse failure as all-defaults. A crash or power loss mid-save (saves happen on every settings toggle, including safe_start's own save at :2467) silently resets rebound shortcuts, interface scale, theme and software_gpu. The project save got atomic temp+rename this week; preferences did not.

**LOW**
- N5 The startup Tripwire is shared by concurrent windows (allowed, lib.rs:153-155). Window 2 overwrites window 1's mark, and window 1's 15 s timer (lib.rs:2408-2412) deletes window 2's, so a crash of window 2 in its first seconds is not recorded. Windows pid reuse can also make `alive()` lie. (reasoned)
- N6 A toast on phones sits under the system bars; see M3 (app.slint:975-981).
- N7 Captions finish matching on English error text (`error.contains("cancel")`, panes/captions.rs:229). Also see L8.
- N8 Doc drift: launch.rs:18 vs the installer name; projects.rs:267-271 "no lock"; lib.rs:2469-2470 SAFETY cites a removed set_var; studio.rs:10212-10215 stacked docs.
- N9 English literals added or kept in user-facing notices (studio.rs:4097, 7820, 8328-8329).

**Not audited (agent reviews missing; treat as open questions)**: today's commits (shortcut-row reset, Audio page shelf, `--software` installer entries, phone shelf pills, the 100+ bundled cards, transition knobs and their undo granularity); keymap layout handling (char vs physical key, AZERTY), Q/W/E bindings while typing; the CC0 sound download (URL, integrity, path traversal); Voiceover mic-denied paths; glide/pinch/hold-repeat timers on Android; Android back button; the project lock on removable or network drives and after a kill; `--mcp`/Automation token strength and bind address; whether `dispatch_event(ScaleFactorChanged)` survives Slint's own resize handling.

### Commits since 15 Oct, short verdicts (only those I read)
- d3f612e vn.json removed: fixes H1.
- e75af9b/1b7e8e6/9197340 saves: per-save temp + ticket ordering (projects.rs:255-330), and close refuses on a failed save then offers quit-without-saving (studio.rs:8345-8368). Sound, and lib.rs has two tests (lib.rs:2599, :2677).
- fa651ef scale: right approach (platform.rs:44-102); runtime correctness unverified.
- b5e6c08/e09d154/64e9dcd safe graphics: works as designed; sticky (N3); smoke leaks the mark.
- 8dfa7bb iOS import: wired; never run here.
- 13f6d4f removed a68641c's contributor feature a day after merging it: churn, not a fix.
- 16a3d13 mask / 915babd stabilize / 244765e canvas: none of them reach copy/duplicate (N2).

### Scores (window crate)
| Aspect | 15 Oct | 20 Oct | Why |
|---|---|---|---|
| Code quality | 6 | 6 | Careful local code, 4 sound non-test expects in 33k. Against: copy paths forgot three new clip fields; clipboard leaks across projects; false SAFETY persists |
| Design architecture | 7 | 6 ↓ | The message/pane split holds, but every feature still lands in `Studio` (106 fields, 247 methods) and `start()` grew to 2 240 lines |
| Design philosophy | 6 | 6 | Platform-native intent is visible (iOS pickers, phone sheets, safe area). Against: features merged then deleted next day; a sticky safe mode instead of a measured one |
| Maintainability | 4 | 3 ↓ | +8.3k Rust in 5 days; studio.rs 12.7k; publish_lanes 396, publish_chrome 353; two 300-400-line splits are cosmetic against that |
| User likeability | — (crate) | 6 | Real gains: iOS import, sheets inside phone bars, home page, shortcuts, VU, voiceover, masks, scale fixed. Losses: wrong-footage paste, copies losing masks, a WARP-forever trap after one bad start, toasts under the nav bar |
| Performance | 6 | 6 | Unchanged hot spots (merge checks per publish, per-frame progress tick); meters tick at 33 ms and stop when quiet (meters.rs:26). Software mode can stick on CPU rendering (N3) |
| Scalability | 5 | 5 | 24-still cap unchanged; per-project id counters make cross-project anything unsafe |
| Security | 6 | 6 (partly unverified) | Save temp uses `create_new` against planted symlinks. Network shelf, MCP and token not reviewed here |
| Testing | 5 | 6 ↑ | +59 tests (155 in 28 files), incl. close/save races and the lock (per commit; not re-read). Still none on copy-field completeness, startup marks or window runtime; smoke never reaches the first frame |
| Documentation | — | 5 | Dense, honest module docs, but drift keeps accumulating (N8) and the docs describe intent more confidently than the code earns |

### Top three to do
1. Clear the clipboard in `release_project` and make copy field-exhaustive (N1, N2). Small, and wrong-footage paste is a correctness bug users will hit.
2. Make safe start one-shot or self-healing, and drop the mark on the smoke path (N3). Write prefs atomically (N4).
3. Pass `disabled` to the export codec Select (M2), inset the toast (M3), and put a run id on captions (L8). Each is a few lines.
