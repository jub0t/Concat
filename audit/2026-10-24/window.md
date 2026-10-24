## Window crate (src/crates/concat), main 2606aff1, 2026-10-24

Method: I read the code at `main` (2606aff1) and `git show` for every commit since c41865e that touches `src/crates/concat` (49 of the 102). I ran `scripts/locales.py --check` and `--same` (python only). I did not build, run cargo or open a device, so runtime behaviour is **reasoned** unless a line says otherwise. Slint behaviour was checked against `i-slint-core-1.18.1` in the cargo registry.

Coverage: the 24 Oct UI commits, the 20-22 Oct fixes for H1/H2/H3, keys, copy/paste and prefs, and PRs #420, #421, #445, #446 and #498 were read in full. The four perf(media) commits, the SVG import and #485 were read for their window-side edits only. The engine reviewer found W-H1 on their own (engine.md E-H1).

### Numbers (measured)
| | 20 Oct (c41865e) | 24 Oct (2606aff1) | Δ |
|---|---|---|---|
| Rust lines (src/) | 33 216 | **34 619** | +1 403 |
| Slint lines (ui/) | 34 957 | **35 929** | +972 |
| studio.rs | 12 790 | **13 180** | +390 |
| `impl Studio` | 9 887 lines, 248 methods | **10 262 lines (2316-12578), 263 methods** | +375, +15 |
| `fn start` (lib.rs) | 2 242 | **2 342** (197-2538) | +100 |
| lib.rs | 2 726 | 2 917 | +191 |
| prefs.rs | 519 | 760 | +241 (atomic save, salvage, tests) |
| Largest fns | publish_lanes 396, publish_chrome 353, key_editor_rows 330, stage_dragged 288, clip_set 264 | publish_lanes **405**, publish_chrome 353, key_editor_rows 330, stage_dragged 288, clip_set 267 | none split |
| Largest .slint | media-pane 2 821, clip-inspector 2 642, start 2 574, lanes 1 926 | **clip-inspector 2 890**, media-pane 2 870, start 2 574, lanes 2 041 | clip-inspector +248, lanes +115 |
| Non-test unwrap/expect | 4 | 4 | host.rs:174, studio.rs:2998, :10685, panes/start.rs:236 |
| `unsafe` lines | 34 | 34 | |
| `#[test]` | 155 in 28 files | **171 in 30 files** | +16 |
| `accessible-*` props in ui/ | 19 in 3 files | 19 in 3 files | none on any control added this period |
| Locales | 25 files | **26 (az)**, 908 keys, --check exit 0 | 985 values still English |

New modules: speed_ramp.rs (331), section-tabs.slint (131), speed-ramp.slint (213). Nothing moved out of `Studio` or `start()`.

### Status of 20 Oct findings
| Finding | Status | Evidence (current code) |
|---|---|---|
| **H1** Safe start makes any early death permanent | **Partly** | Software now lasts one build, not for good: `prefs.rs:302-321` `settle_start`, keyed by `machine::build()`. `--smoke` no longer arms the mark (`lib.rs:214-216`). The notice shows on every launch it holds (`lib.rs:2664-2666`). Still open: any death of any cause in 15 s trips it (no crash signature). Errors returned before the loop still drop the mark through `Tripwire::drop` (`jobs.rs:169-172`; `select_backend(...)?` lib.rs:223-288, `Host::start` :290-296). Concurrent windows still share one mark, and the 15 s timer deletes it by path (`lib.rs:2496-2501`). New: on Android it now breaks export (W-H1) |
| **H2** Android Back quits; nothing saved on the way out | **Partly** | Back takes Escape's road for window sheets (`app.slint:512-541`). With nothing on top it closes the project (`:547-553`). The loop's end saves on Android (`lib.rs:2521-2533`). Not covered: the phone editor's own sheets and the voiceover panel (W-H2). Slint still forwards no Pause or Stop, so a background kill loses up to the 1.5 s autosave |
| **H3** Phone timeline moves clips on a tap | **Partly (body fixed)** | A press under a finger is nothing until it is a tap, a swipe past 8 px, or a 400 ms hold (`lanes.slint:585-600, 630-644, 669-686`). The floor's slop is 8 px (`:1282-1287`). A pinch's first finger no longer selects. Still: the selected clip's edge trims at once on the 4 px desk threshold (`lanes.slint:592`, `:657-658`). Targets are unchanged (`Theme.h` 28 px, `theme.slint:148`) |
| Destructive chords while typing | **Fixed** (for the keymap) | `lib.rs:1712` drops every action but Save when `typing()` (`:2544-2551`) or `sheet_open()` (`studio.rs:9195-9201`). Ctrl+W, Ctrl+O and Ctrl+Q are KeyBindings (`app.slint:476-495`) and still fire from a field |
| No guard while a sheet is open | **Partly** | `sheet_open` covers Export, Settings, Project, Relink and Confirm. Captions, Speech, Voiceover, the start sheet and the phone's sheets are not in it |
| Capture `reserved()` unreachable; alias holes; letters dead on non-Latin layouts | **Unchanged** | keymap.rs changed only its copyright line; the KeyBindings above still match first |
| Copy/paste/duplicate drop clip fields (N2) | **Fixed** | `match_command` (`studio.rs:2241-2251`) sends `MatchClip`, which copies the whole clip but its place (`concat-project/.../properties.rs:330-360`). Titles, shapes and now layers go through it too (`studio.rs:8340-8385`). A test covers mask, stabilize, opacity and transition (`concat-project/src/lib.rs`, `a_matched_clip_takes_everything_but_its_place`) |
| Paste after switching projects (N1) | **Partly** | The clipboard carries the media item, and `held_here` maps it by path or adds it to the bin (`studio.rs:8288-8326`). `Enhanced { original, copy }` still holds the old project's media ids (W-M5) |
| Prefs non-atomic, any parse failure resets all, world-readable (N4) | **Fixed** | Temp file then rename (`prefs.rs:352-365`). Written 0600 on Unix (`:391-401`). One bad value costs only itself (`salvage`, `:369-389`), and the bad file is set aside (`:337`). Residuals are Low (below) |
| Interface scale not applied at launch | **Fixed** | `set_ui_scale` runs before the window (`lib.rs:203-211`). `apply_ui_scale` retries 40 × 50 ms until winit has a window (`platform.rs:66-75`) |
| Toast not inset on phones (M3/N6) | **Unchanged** | `toast.slint:88`, 16 px off the window's bottom |
| Select/SegmentedControl/Switch assign their own bindings | **Unchanged, new instance** | `select.slint:140`, `segmented-control.slint:47`, `controls.slint:563-565`. The new effect-card switch is one (W-M1) |
| Export codec picker without `disabled` (M2) | **Unchanged** | `dialogs/export.slint:326-334` passes no `disabled` |
| Duplicate is two undo steps (M4) | **Unchanged, wider** | `studio.rs:8183` `place_copy(..., None)`. Titles, shapes and layers are now two steps too, where titles were one (`:8381-8384`) |
| Captions cancel/restart collide (L8); English error matching (N7) | **Unchanged** | `panes/captions.rs:229` `error.contains("cancel")` |
| `--smoke` stops before the window (#17) | **Unchanged** | exits at `lib.rs:2454-2473`, before `app.run()` (:2509) |
| Unknown argument refuses to start (#25) | **Unchanged** | `launch.rs:75-79` |
| Event-loop failure exits with no dialog | **Unchanged** | `lib.rs:2513-2519` |
| Quit during an export kills it without asking | **Partly** | `stop_exports` cancels cleanly and tells API callers (#431, `lib.rs:2586-2620`). It still does not ask |
| RTL; accessibility names | **Unchanged** | No layout direction anywhere. The 19 `accessible-*` properties are the 20 Oct ones |
| False SAFETY comments (L1, 12th) | **Unchanged + one more** | `lib.rs:2657` still cites "the scale above", which sets no env. The new `platform.rs:451-454` says "single-threaded" right after `Gpu::acquire()` (:436) has opened a device (lavapipe starts threads) |
| English literals in notices (11th) | **Unchanged** | `studio.rs:4238`, `:8719-8720`, despite 72ba60a2 "every string the interface draws goes through the lookup" |

Tally: 4 fixed, 7 partly, 11 unchanged (2 of them wider).

### New findings
**HIGH**

- **W-H1. On Android, one early death means every export fails until the next build.** 7df9f8f2 runs safe start on Android (`lib.rs:2643-2645`). It sets `CONCAT_GPU=software` process-wide (`:2656-2662`), and the comment says that means "OpenGL ES in place of Vulkan (the Skia patch reads it), not the CPU". `concat-render/src/gpu.rs:828-833` reads the same variable as *software adapters only* for every compositor. Android lends the export no device (`platform.rs:681-684` returns `None`), so the export calls `best_compositor` → `WgpuCompositor::new()` (`concat-export/src/lib.rs:1573-1574, 1693`). With `force_fallback_adapter` there is no adapter on a real phone, so the export ends with "no GPU or software renderer". Cards go the same way (`concat-host/src/cards.rs:411-413`). Android has no GPU switch to undo it (`panes/settings.rs:858`). An early death does not need a bad driver: a swipe from Recents in the first 15 s, or the system killing the app behind the file picker, leaves the mark. CI's emulator runs SwiftShader, a CPU adapter, so CI would pass. *Reasoned; corroborated independently by engine.md E-H1, which adds that the monitor draws nothing.* Fix: on Android, give the Skia patch its own variable, or have `own_api_first` ignore `CONCAT_GPU` there.
- **W-H2. Android Back on the phone editor's own sheets throws you out of the editor.** The phone's Media, Audio, Text, Effects, Filters, Mask, Canvas, Speed and other sheets live in `PhoneEditor`'s private `property <string> sheet` (`phone/editor.slint:343`). Nothing in `phone/` handles a key. The window's handler cannot see that state (`app.slint:512-541`), so Back falls through to `app-menu-selected("close-project")` (`:547-550`). Scenario: on Android, open Mask on a clip and press Back. The project saves and closes, and you are on the launch screen. The voiceover panel is missing from the list too: Back during a take closes the project, and the take is kept on disk but not laid on the timeline (`studio.rs:8813-8815`). 0aa1a502's message says "Back closes the sheet on top", which is untrue for the phone's commonest sheets. Fix: a `PhoneEditor.back() -> bool` that closes its sheet, called first; add `Editor.voiceover-open`.

**MEDIUM**

- **W-M1. The effect card's switch goes stale after Undo.** `Switch { checked: item.on; toggled => root.toggle(index) }` (`clip-inspector.slint:363-366`). The Switch writes `root.checked = !root.checked` itself (`controls.slint:563-565`), which ends the binding, and the rows are kept and `sync`ed in place (`studio.rs:803-818`). Scenario: switch an effect off, then Ctrl+Z. The effect is on and its knobs are bright (`opacity: item.on`), but the switch still shows off. The next click turns it "on" visually and off in the document.
- **W-M2. Fold state belongs to the row, not the effect.** `property <bool> open: true` per repeated row (`clip-inspector.slint:331`), on a model updated in place. Fold the first effect and press its ↓: the effect moves down open, and the effect that took its place shows folded. Selecting another clip keeps the folds by index.
- **W-M3. A sound layer made from a Clean effect cannot be seen, tuned or taken off.** The audio shelf carries `concat.denoise`, `concat.loudness` and `concat.enhance-voice` (category Clean). A sound layer's Audio tab is Layer + AudioEffects (`config-pane.slint:314`), and AudioEffects hides those three ids (`:798`) because a sound clip's Sound section has switches for them. A layer has no Sound section, so drag Denoise onto the lanes and the layer's only link appears nowhere. A Clean tile clicked with a sound layer selected disappears the same way.
- **W-M4. The Effects tab appears on any clip whose colour was touched.** b5131917 shows Effects only "on a clip that already carries a chain (an older project, a template, the API)" (`config-pane.slint:251-255`). But the first Adjust knob inserts `concat.adjust` at the head of `video_effects` (`studio.rs:4755-4763`), and Basic › Mode › Chroma key adds `concat.chroma-key` (`clip-inspector.slint:1230-1238`). Both make `has-chain` true, mid-drag for the knob. The new tab then holds an "Adjust" card whose knobs are hidden (`studio.rs:1879-1883`) and whose trash removes the grade. On an image, the header flips from the plain heading with the clip's name to a segmented control. Also, desktop and phone now disagree: the phone still applies effects to the clip (`phone/editor.slint:1229-1230`).
- **W-M5. A clip pasted into another project switches to the wrong file when Enhance is toggled.** `MatchClip` keeps `enhanced` (`properties.rs:330-360`). `held_here` remaps only `media_id` (`studio.rs:8288-8326`). `SetClipEnhanced` then sets `media_id` to `enhanced.original` or `.copy` (`concat-project/.../clips.rs:498-513`), which are the source project's ids. Fix: remap or drop `enhanced` in `held_here`.
- **W-M6. Esc in the font picker's search no longer closes it, and the next key goes to the timeline.** SearchField now accepts Escape and calls `Focus.release()` (`controls.slint:763-770`), which focuses the window's `keys` scope (`app.slint:379-381`). Slint closes the top popup only on an *unhandled* Escape (`i-slint-core-1.18.1/window.rs:1372-1395`). The font list is a `PopupWindow` whose search is focused on open (`font-picker.slint:41-46, 98-101`). After Esc it stays open, the keyboard sits on the editor behind it, and `typing()` and `sheet_open()` are both false, so Q, W or Delete edit the timeline under an open popup. Enter has the same effect via `accepted`.
- **W-M7. Every window focus stats every media file on the UI thread.** PR #421: `check_missing_media` → `missing_media()` on each focus-in (`lib.rs:256-264`, `studio.rs:8681-8689`). On a Windows project with footage on a share that has gone away, each Alt-Tab back waits on SMB timeouts. *Reasoned.*

**LOW**
- Keyboard reach lost in today's UI. The eye was an `IconButton` with a FocusScope and a name; the `Switch` has neither. The audio library's shelves were a SegmentedControl with arrow keys; the pills (`clip-inspector.slint:565-636`) and the card fold (`:351-354`) are TouchAreas. Bypass, fold and shelf choice now need a pointer.
- Stale copy. `clipInspector.chainRunsTopBottom` still says "The eye bypasses one link" in all 26 locales (shown at `clip-inspector.slint:311`). `configPane.noEffectClipYet` still says "Pick one from the Library's Effects page", which now lays a layer instead. `Param.rail`'s doc says "not for a list of knobs" (`controls.slint:345-347`), and every effect knob now uses it.
- Dead code and stale comments. `set_phone` still sets groups to -1 "a desk's strip starts on the first" (`studio.rs:9255-9262`), now the default for all. `library[1].category = "All"` duplicates group -1 (`:2455-2457`).
- The audio pills do not wrap or scroll: about 330 px of pills against a 240 px seat minimum (`dock.rs:9`). A narrow inspector clips the last shelves. *Reasoned.*
- A sound layer selected on the phone gets no clip tools: `phone/editor.slint:350-360` has no `sound-layer`. The phone cannot make one, but a desk project can bring one.
- `typing()` reads `slint::private_unstable_api` (`lib.rs:2544-2551`), so a Slint bump can break the keyboard guard silently.
- Prefs residuals. The `.json.unreadable` copy keeps the old file's mode through `fs::copy`, and it holds the API token (`prefs.rs:337`). A rename refused on Windows (AV or another reader) drops the save without a log.
- `start.safeModePhone` (new, #507) is English in most locales. The localized "Untitled project" (`start.rs:207-225`) treats an English-named untitled project as typed after a language switch.
- Doc drift outside the crate, from 5296d98c: `concat-host/src/cards.rs:506-509` now stacks the old "Every picture package has a card…" doc on `draw_every_card`.

### Commits since c41865e, short verdicts
| Commit | Verdict |
|---|---|
| f26ebf64 per-build safe start (#476) | Right shape; still trips on any death; see H1 |
| 7df9f8f2 Android safe start (#507) | **Breaks Android export after one early death** (W-H1) |
| 3f6db386 software adapter → Skia CPU + own device (#503, #480) | Sound idea; SAFETY wording false (above) |
| 0aa1a502 Android Back + save on Destroy | Half: window sheets yes, phone sheets and voiceover no (W-H2) |
| b467155c hold to move | Good; edges still 4 px |
| e2be568a keys out of fields/sheets | Good; KeyBindings and four sheets not covered |
| 1d04d147 MatchClip, paste by file | Good design (whole-clip copy, never on the API); `enhanced` not remapped (W-M5); duplicate still two steps |
| 4b1b2b05 prefs atomic, 0600, salvage | Good, with tests |
| 816a8f0e scale at startup (#462) | Fixed |
| c495a8ea / #498 ⌘Q saves (#437) | Correct; a failed save there is only logged, which is unavoidable at terminate |
| c631c459 phone delete ripples main lane (#497) | Read; matches phone editors; desk unchanged |
| b3054fae field keeps caret in view (#506) | Read; no defect found |
| 87fd6781 / #421 relink on focus (#409) | Works; UI-thread stats per focus (W-M7) |
| 80561bd2 / #420 speed curve | Read speed_ramp.rs and the Studio edits; echo-then-commit like other knobs; no defect found |
| 77f9457d / #445 early return republish | Correct |
| 4ffa3212 / #446 quit cancels export | Correct; bounded at 10 s |
| 0acb767b title Effects tab (#484) | Superseded two commits later by "only with a chain" |
| bb30386b SectionTabs | Bar and index are clamped (`config-pane.slint:666-668`); arrows wrap; FocusScope present; no accessible role (none anywhere) |
| 22639548 effect cards, audio pills | Looks better; W-M1, W-M2, keyboard reach lost |
| 8dddc279 / b5131917 effects as layers, tab only with a chain | Product call is defensible (CapCut does both); execution leaks (W-M4); phone and desk now differ |
| 0a45c5a5 SearchField Enter/Esc | W-M6 |
| 6466b9f7 All rows, default group -1 | Correct (no `groups[group]` lookups in ui/); dead phone defaults left |
| 599157e7 Preview chip, end_audition | Works (ends on clip press, deselect, Esc, cross, project close); locale keys added in all 26 |
| 5296d98c square cards | Fine; edition bump redraws caches once |
| 2606aff1 sound layers | Window side consistent (lanes, inspector, copy); W-M3 |

### Scores (window crate)
| Aspect | 20 Oct | 24 Oct | Evidence |
|---|---|---|---|
| Code quality | 6 | 6 | `MatchClip` and atomic prefs remove two bug classes. Today's cards reintroduce a self-assigning control (W-M1) and row-indexed state (W-M2), and an env var means two things in two crates (W-H1) |
| Design architecture | 6 | 6 | A whole-clip copy command that never crosses the API is the right structure. `Studio` still absorbs everything: 263 methods, 10.3k-line impl, `start()` 2 342 lines |
| Maintainability | 3 | 3 | +1.4k Rust and +1k Slint in 4 days with no split. publish_lanes is now 405 lines and clip-inspector.slint 2 890. Copy drift shows up in the same commit that changes the UI |
| User likeability (UI/UX) | 6 | 6 | Real gains: hold-to-move, Back on sheets, safe start that heals, Preview chip, All shelves, square cards. Against them: Android Back ejects from phone sheets and an Android early death kills export, the Effects tab appears unasked, Esc misbehaves in the font picker, and keyboard reach was lost |

### Before a release
1. W-H1. Android export after a safe start. One line of scope (`cfg(not(target_os = "android"))` on the `CONCAT_GPU` read in `own_api_first`, or a separate variable).
2. W-H2. Back closes the phone editor's sheet and the voiceover panel before it closes the project.
3. W-M1 and W-M6 are small and user-visible. Bind the card switch one way with `changed`, or take the Switch's self-assign out. Let Esc through when the field is inside a popup.
