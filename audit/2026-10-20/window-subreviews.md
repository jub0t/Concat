# Window sub-reviews, 20 October 2026

Four read-only reviews the window reviewer ran alongside
[`window.md`](window.md), which reported partial without them. Code at
`baaeb2a`. "Verified" means traced in the source, including Slint 1.18.1
and winit 0.30.13 in the cargo registry; nothing here was run on a device.
Where a finding was fixed later today, the fixing commit is named.

## 1. Today's commits

| Commit | Finding | Severity | Status |
|---|---|---|---|
| ce64dae transition knobs | Knob rows were a fresh `ModelRc` on every publish (`studio.rs:9044-9045`), and Slint drops every repeated row when the model handle changes (`i-slint-core` `model/repeater.rs:663-672`), so a scrub stopped after one step | High | Fixed in d4ced41 (kept model + `sync`) |
| ce64dae | One undo step per value change (`self.apply`), up to 100 steps a drag against `UNDO_DEPTH = 200` | Medium | Fixed in d4ced41 (`apply_within`); gestures on the same knob are never ended, so two drags can fold into one step |
| ce64dae | Params never validated outside the UI setter: API/MCP JSON reaches the shader and the file | Low | NaN refused and tidied in d4ced41; range clamp at render still open |
| ce64dae | No transition panel on the phone | Low | Open |
| ce64dae | Tests: one export copy test, one GPU test that skips without an adapter unless `CONCAT_REQUIRE_GPU` | Low | Open |
| 64e9dcd `--software` | `--software --project X` refused (`launch.rs:76`) | Low | Open |
| 64e9dcd | Non-atomic `Preferences::save` (`prefs.rs:267-271`): a second instance can write `software_gpu=false` back | Low-Medium | Open |
| 64e9dcd | `CONCAT_GPU=software` leaks into the Linux update relaunch (`updates.rs:620-631`) | Low | Open |
| 64e9dcd | Doc drift: "software drawing" vs "safe graphics" | Low | Fixed in d4ced41 |
| 64e9dcd | No Linux `.desktop` action | Low | Open |
| f2d0637 bundled cards | Keyed by id only: a changed shader shows the old shipped card until `scripts/bundle-cards.sh` is re-run | Low | Open |
| f2d0637 | ~100 JPEG decodes on the UI thread on first shelf build, again after every reload | Low (unmeasured) | Open |
| b18c2fe / a9b2df6 FXC pin | Correct; a system-wide `WGPU_DX12_COMPILER=dxc` brings #418 back | Low | Open by design |
| be60358 shortcut reset | Correct (`input_items.rs:74-107`) | — | — |
| f121747 Concat sounds removed | `docs/AUDIO_LIBRARY.md:3-8,39-40` still describes the shelf; dead UI/Rust left (`audio-page.slint:71-80`, `studio.rs:11052`, `panes/sounds.rs:176`) | Low | Open |
| a1fd411 phone pills | A drag that starts on a pill never reaches the strip's TouchArea (`media-pane.slint:1440-1442`), so off-screen shelves stayed unreachable by touch | Medium | Fixed in d4ced41 (pills forward drags) |
| a1fd411 | Phone opened on group 0, not All | Low | Fixed in d4ced41 |
| a1fd411 | `sounds.all` reused, masculine agreement in es/pt-BR/fr/it | Low | Fixed in d4ced41 (`common.all`) |

## 2. Phone shell

- **High, Android Back quits the app**, even with a sheet open or an export
  running: nothing handles `Key.Back` (`app.slint:504-539` handles Escape
  only), and Slint finishes the activity on an unhandled Back
  (`javahelper.rs:788-811`).
- **High, a touch on a clip moves it at 4 px** (`lanes.slint:591-592`;
  Android slop is ~8 dp). Every press selects; past 4 px it is a committed
  `MoveClips` with an undo step (`studio.rs:4804-4818, 4978`). Swipe-scrub
  only starts on empty lane or the 24 px ruler (`editor.slint:726`).
- **Medium-High, pinch selects and can move clips**: the first finger's
  press already selected; Slint synthesises a release with the second
  finger's id (`input.rs:2271-2276`).
- **Medium, toast not inset**: under the nav bar / home indicator
  (`app.slint:975-981`, `toast.slint:87-88`). No soft-keyboard inset
  anywhere (Slint passes it, `javahelper.rs:814-830`; the app never reads
  it).
- **Medium, self-assigning controls unchanged**: `Select`
  (`select.slint:140`), `SegmentedControl` (`segmented-control.slint:47`),
  `Switch` (`controls.slint:561`). Concrete: Captions shows Top, Rust holds
  Bottom after a reopen (`captions.slint:226,235`, `captions.rs:409-419`).
- **Hit targets** under 44-48 px throughout: transport 36 px, Play 40 px,
  zoom chips 34, shelf pills 24, `Theme.h` 28, `Switch` 30×17; Android has
  no extra UI scale (`platform.rs:90`).
- Hold-to-repeat keeps firing after the finger slides off
  (`controls.slint:61-78`). No RTL layout for ar/fa/he/ur. No accessibility
  properties in `ui/phone`. No Slint UI tests anywhere.

## 3. Keymap, Audio page, Voiceover

- **Medium, destructive chords fire while typing**: Slint's `TextInput`
  ignores non-standard Ctrl chords (`items/text.rs:1120`), so they walk up
  to the keymap. Ctrl+B in a title field splits the title clip, Ctrl+I
  opens Import; in read-only fields Backspace deletes the selected clip.
- **Medium, no guard for open sheets**: with Settings/Export open, Space,
  Delete and Q/W act on the timeline behind (`lib.rs:1621`).
- **Medium, capture's `reserved()` unreachable**: KeyBindings match first
  (`input_items.rs:723-745`), so Ctrl+W closes the project while a
  Shortcuts row is listening; Escape closes Settings, so "Escape declines"
  is dead.
- **Medium, bindings by character, not physical key**: every letter
  shortcut is dead on Cyrillic, Greek, Hebrew, Arabic and Persian layouts
  (`winitwindowadapter.rs:1324`), and the app ships ru/uk/fa/ar/he.
- Alias conflict holes in `bind`/`reset` (`keymap.rs:257-313`).
- **Concat sounds trust anchor**: v0.2.8 still ships the "Load Concat
  sounds" button against `github.com/concatapp/assets`. Verified at audit
  time: `ConcatApp` is an organization the maintainer's account belongs
  to, so not squattable. Still: no signature, mutable `main`, digest in the
  same manifest, no download size cap (`models.rs:224-305`), Cancel never
  set (`panes/sounds.rs:113,333`), `adopt_sound` copies and probes on the
  UI thread (`panes/sounds.rs:584-637`).
- **Medium, Android microphone permission dead end**: `askMicrophone`
  returns before the result, with no `onRequestPermissionsResult`
  (`ConcatFiles.java:169-185`); a second denial has no route to settings.
- Voiceover takes: retake orphans pile up in `project/audio`; closing mid
  take drops the take from the project (`studio.rs:8414`).
- Locales: `studio.merge*` English in 21 locales, `settings.scale*` in 22.

## 4. Startup, close, project lock

- **High, File › Open with a project open dropped it unsaved and leaked
  its lock** (`start.rs:427-457` → `studio.rs:8162-8212`). Fixed in
  2abfcf7.
- **Medium, Android saves nothing when the activity ends**: Slint ends the
  loop on `MainEvent::Destroy` and ignores Pause/Stop/SaveState
  (`androidwindowadapter.rs:271-273`); `leave(0)` follows with no
  `close_project` (`lib.rs:2433-2434`).
- **Medium, safe start treats any early death as a driver crash, for
  good** (`lib.rs:2454-2475`, `SAFE_AFTER` 15 s): a Task Manager kill, a
  logoff (winit 0.30.13 handles no `WM_ENDSESSION`), or a panic opening a
  project. Since b5e6c08 that also puts exports on WARP + FXC. The notice
  shows once. Errors returned before the loop (`lib.rs:253,259,265`) drop
  the mark instead.
- **Medium, `--smoke` leaves the start mark** (`lib.rs:2381,2385` exit
  without clearing it): the next real launch is forced into software.
- **Medium, a remembered interface scale is not applied at launch**: the
  zero-delay timer fires in `NewEvents(Init)` before the winit window
  exists (`lib.rs:2416-2419`, slint winit `event_loop.rs:213-226`), so
  `apply_ui_scale` returns early (`platform.rs:65-67`). Reasoned, not run.
- **Medium, the project lock opens unguarded on any I/O error**
  (`concat-api/src/lib.rs:183-189`): OneDrive or antivirus sharing
  violations on Windows, ENOLCK on network mounts.
- Event-loop failures exit with code 1 and no dialog (`lib.rs:2425-2431`);
  phones have no failure report and no safe start (`platform.rs:537`,
  `lib.rs:2455`).
- Low: a bare path argument (drag a folder onto the shortcut) refuses to
  start (`launch.rs:74-79`); `prefs.json` is world-readable on Unix and
  non-atomic (`prefs.rs:267-272`) though `discovery.rs:11-14` claims
  owner-only; Quit-without-saving during a version switch never starts the
  new build; closing during an export kills it without asking.
