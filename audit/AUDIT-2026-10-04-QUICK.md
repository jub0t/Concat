# Concat audit, 4 Oct 2026 — the quick version

Same audit as `AUDIT-2026-10-04.md`, one screen. Each bar has 10 slots:
🟢 filled = good, ⚪ empty = missing. Severity: 🔴 critical · 🟠 high · 🟡 medium · ⚪ low.
Arrows are against the 28 Sep audit. Every crate was read in full; nothing is provisional.

## 🎯 Verdict

**A good week for the window, a lost week for the gate.** Six days and 60 commits after 0.2.5, the app gained a razor, a scrollbar, a volume line, keyboard menus, Save Frame and Save Audio, shapes, an export sheet with a picture, hardware encoders, and an export that survives a dead GPU, each well made at its seam. But `main` has not been green since the release: fmt has been red on every push and nothing behind it has run for 71 commits; none of the seven steps recommended on the 28th was taken; and three of the old findings got worse while the engine sat still. No new critical. Four new highs: the ignored gate, a `[[replaces]]` that lets a user package hijack a built-in, preview sound off by the stream start on MTS files, and eight locales a fifth English.

## 📏 Size

| | Lines | Δ vs 28 Sep |
|---|---:|---:|
| Rust, all crates | 87 376 | +3 800 |
| Slint UI | 28 796 | +1 450 |
| WGSL, packages | 1 764 | +20 |
| Tests (`#[test]`) | 676 | +26 |
| Largest file `concat/src/studio.rs` | 10 029 | +568 |

## 📊 The whole app

| Aspect | Bar | 28 Sep | Today |
|---|---|:-:|:-:|
| 🧹 Code quality | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 7 | 6 ↓ |
| 🏛️ Architecture | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 |
| 🧭 Design philosophy | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 7 | 6 ↓ |
| 🔧 Maintainability | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 6 | 5 ↓ |
| 😊 User likeability | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 |
| ⚡ Performance | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 |
| 📈 Scalability | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5 | 5 |
| 🔐 Security | 🟢🟢🟢🟢⚪⚪⚪⚪⚪⚪ | 5 | 4 ↓ |
| 🧪 Testing | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 7 | 6 ↓ |
| 📚 Docs | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 7 | 6 ↓ |
| 🤖 CI and release | 🟢🟢🟢⚪⚪⚪⚪⚪⚪⚪ | 6 | 3 ↓ |

## 🧱 Per crate (average of its scales)

| Crate | Lines | Δ | Bar | Avg | One thing |
|---|---:|---:|---|:-:|---|
| concat-vision | 2 760 | 0 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.2 | 🟢 untouched since the 23rd · ⚪ runtime wrapper untested |
| concat-text | 1 974 | +734 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.1 | 🟢 faces mmapped once behind `Arc` · 🟡 emoji are boxes on a Mac |
| concat-effects | 7 096 | +6 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.4 ↓ | 🟢 344 probes · 🟠 `[[replaces]]` can hijack a built-in |
| concat-core | 2 854 | +25 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.3 | 🟢 `hold` is one flag in two places · ⚪ dead `Project`, i64 rate panics |
| concat-server | 1 376 | 0 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.0 | 🟢 constant-time token · 🟠 fd leak, zero churn |
| concat-api | 2 351 | +1 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.9 ↓ | 🟢 real roots check · 🟡 four window-only operations now |
| concat-project | 9 465 | +273 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.9 | 🟢 shapes tidied in the one right place · 🟡 0.2.5 deletes shapes |
| concat-render | 8 124 | +80 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.6 | 🟢 parity for 10 plan features · 🟡 a timed-out trial kills the monitor |
| concat-media | 9 494 | +453 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.8 | 🟢 37 `unsafe`, 37 SAFETY lines · 🟠 playback audio ignores the stream start |
| concat-speech | 3 109 | +15 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.4 ↑ | 🟢 one accelerator switch, one fallback · ⚪ `DirectML.dll` not staged |
| concat (Slint) | 28 796 | +1 450 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.5 | 🟢 0 ⌘ glyphs now · 🟡 no dialog a keyboard can press |
| concat-cli | 514 | 0 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.2 ↓ | 🟢 token from env · 🟡 no logger, so export diagnostics vanish |
| concat-export | 3 702 | +204 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.1 | 🟢 device loss recovered, no partial file · 🟡 hold plays keyed effects early |
| concat-host | 11 869 | +329 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 5.8 | 🟢 logs with a level of their own · 🟠 update trust chain unchanged |
| concat-android | 335 | 0 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.0 | 🟢 portrait locked · ⚪ 0 tests, ships Chatterbox |
| concat (Rust) | 21 261 | +1 697 | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5.4 ↓ | 🟢 epoch gate on 10 of 12 workers · 🟡 `studio.rs` 10 029 lines, a full publish per frame |

## 🎛️ Per feature

| Feature | Bar | 28 Sep | Today | State |
|---|---|:-:|:-:|---|
| Launcher | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ |
| Import and bin | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ Wayland drop |
| Timeline editing | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ ★ razor, ★ scrollbar |
| Undo / redo | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ |
| Effects and filters | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ shapes round, glows past white |
| LUTs | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ |
| Relink | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ |
| Themes | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ lime default |
| Transitions | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 8 | 7 ↓ | ⚠️ ★ dissolve hold; keyed effects play early |
| Colour and HDR | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Scopes | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ verified this time |
| Colour grading | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Titles | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ⚠️ ★ wraps at the frame; emoji boxes on macOS |
| Keyframes | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Crop, blend, masks | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Cutout | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Captions | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ⚠️ words off by the stream start on MTS |
| Text-to-speech | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ ★ DirectML with CPU fallback |
| Templates | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Export | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ⚠️ ★ survives a dead device; rate ladder fixed 24/30/60 |
| Hardware decode | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| ★ Logs | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | – | 7 | ✅ a file per run, bursts folded |
| ★ Save Frame / Save Audio | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | – | 7 | ✅ epoch-gated; Save Audio window-only |
| Custom packages | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 7 | 6 ↓ | ⚠️ trial at defaults only; `[[replaces]]` unfenced |
| ★ Shapes / Stickers | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | – | 6 | ⚠️ not editable; 0.2.5 deletes them |
| ★ Volume line | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | – | 6 | ⚠️ can drop typed title words |
| ★ Hardware encode | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | – | 6 | ⚠️ never exercised on a chip in CI |
| ★ Keyboard menus | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | – | 6 | ⚠️ stalls past four dead rows |
| Languages | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 8 | 6 ↓ | ⚠️ 8 locales a fifth English; no RTL |
| Remote API | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 | ⚠️ one line changed; fell behind the window |
| Enhance | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 | ⚠️ copies in `cache/` |
| Phone shell | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 | ⚠️ ★ swipe to close; its own 44 px rule broken |
| ★ Windows data folders | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | – | 6 | ⚠️ a failed move is silent |
| Proxies | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5 | 5 | ⚠️ HDR copy under an SDR name |
| Playback and audio | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 6 | 5 ↓ | ⚠️ sound ahead of picture on MTS |
| Self-update | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5 | 5 | ⚠️ ★ `.msi` path; same trust chain |
| Accessibility | 🟢🟢⚪⚪⚪⚪⚪⚪⚪⚪ | 2 | 2 | ❌ two `accessible-*` in 28 796 lines |

## 🔁 The 28 Sep findings today

**0 closed · 4 partly closed · 14 open**, three of them worse than described.

| # | Was | Now |
|:-:|---|---|
| 1 | update trust chain | 🟠 open; Windows signed only when a secret is set |
| 2 | API rooted at the home folder | 🟠 open, byte-identical |
| 3 | fd leak per connection | 🟠 open, byte-identical |
| 4 | Clear cache deletes reversed media | 🟠 open |
| 5 | 0.2.4 documents open changed | 🟠 open |
| 6 | embedded API's second GPU device | 🟡 open |
| 7 | `[[replaces]]` drops keys | 🟠 **worse**: user packages' tables do fire, and can name a live built-in |
| 8 | HDR proxies not regenerated | 🟡 **worse**: an HDR copy written under the SDR name |
| 9 | Hub in line, no `catch_unwind` | 🟡 open |
| 10 | downgrade one click | 🟡 open |
| 11 | no frame ceiling | 🟡 **sharper**: an i64 rate panics the export thread |
| 12 | bin cloned per snapshot | 🟡 open |
| 13 | export one thread; 400 pieces | 🟡 partial: the cut moved to one place in concat-export |
| 14 | font by path, silent | ⚪ open |
| 15 | small project correctness | ⚪ partial: no-op undo steps fixed; `ShapeStyle` joins the no-`extra` list |
| 16 | server hygiene | ⚪ open |
| 17 | host hygiene | ⚪ partial: `rpm -q` memoised |
| 18 | carried hygiene | ⚪ partial: ⌘ glyphs gone, `mobile.yml` sentence was true; `ui/demo/`, `core::Project`, speed range remain |
| – | two locale-flaky tests | ⚪ open, one failed again today |

## 🆕 New findings

| # | Sev | What | Where |
|:-:|:-:|---|---|
| 1 | 🟠 | `main` red at `cargo fmt --check` since 28 Sep; clippy, tests and perf have not run on CI for 71 commits; CI builds on 1.99, the pin says 1.93 | `ci.yml:93` · 10 files, 22 sites |
| 2 | 🟠 | A user package's `[[replaces]]` can name a live built-in and rewrite every link to it at open, keys dropped, no notice | `catalogue.rs:1050,986` · `manifest.rs:91` · `session.rs:105` |
| 3 | 🟠 | Preview sound and the transcript ignore the stream start; the export does not: sound runs ahead on MTS files | `samples.rs:91,176` vs `audio.rs:424` |
| 4 | 🟠 | Eight locales have 128–197 values that are the English string; the check compares keys only | `scripts/locales.py:134` · `de.json` |
| 5 | 🟡 | Shapes: 0.2.5 opens them as Video, drops them, deletes them on save at the same `DOCUMENT_VERSION`; unknown kinds become squares; not editable after placement; 17 of 44 commands outside the round-trip test | `model.rs:1159,1184` · `doc.rs:41,266` · `commands/mod.rs:143` |
| 6 | 🟡 | Install trial runs at the knobs' defaults; a timed-out trial sets `dead` on the monitor's own compositor | `catalogue.rs:489` · `gpu.rs:1869` · `preview.rs:120` |
| 7 | 🟡 | Dissolve hold plays keyed effects over the pre-roll; keys on a clip with a handle play early | `concat-export/src/lib.rs:776,803,1478` |
| 8 | 🟡 | Export rate is a fixed 24/30/60 ladder; a 25 fps cut exports at 30 by default | `studio.rs:154` · `export.slint:363` |
| 9 | 🟡 | A volume-line drag within 120 ms of a title blur drops the typed words | `studio.rs:4446,6213,4634` |
| 10 | 🟡 | Art caches never released across projects; `spawn_art`, `spawn_strip` and `save` not epoch-gated; `close_project` never cancels an export | `studio.rs:6626,6739,2822,2310` |
| 11 | 🟡 | A full publish per monitor frame during playback | `host.rs:243` · `monitor.rs:257` |
| 12 | 🟡 | No dialog button or card is keyboard-reachable; Escape closes no modal | `modal.slint:17,168` |
| 13 | 🟡 | Six strings and the effects category bar bypass `I18n` | `media-pane.slint:1332,2197,2239` · `tray.slint:182` |
| 14 | 🟡 | Emoji paint as boxes in titles on macOS | `concat-text/src/lib.rs:432,530` |
| 15 | 🟡 | No Unreleased section; version still 0.2.5; ~20 user-visible changes unrecorded | `CHANGELOG.md` · `src/Cargo.toml:6` |
| 16 | 🟡 | +3 800 lines, 26 tests; `tests/export.rs` untouched | `git log --numstat` |
| 17 | 🟡 | README sells an MCP API that does not exist | `README.md:62,79,114` |
| 18 | 🟡 | The CLI installs no logger; device-loss diagnostics vanish through `serve` | `concat-cli/src/main.rs` |
| 19 | 🟡 | API one line changed; Save Audio, log level, CBR and the hardware-encoder choice are window-only; `export.run` cannot refuse a chip | `concat-export/src/lib.rs:1277` · `concat-api/src/lib.rs:568` |
| 20–28 | ⚪ | NaN check after the clone; scopes freeze on a failed map; `transitions.rs` oracle unchecked; no fallback once a chip encoder opened; "· hardware" for Media Foundation; greys-only colour tests; silent data-folder move; `msiexec` on a hand-unpacked zip; poster under the wrong name; extension swapped after the overwrite prompt; four copies of the inspector ranges; menu walking stalls past 4 dead rows; 28 colours outside `theme/`; phone 34/40 px buttons; `in-chain` capped at 8; gRPC without caps; Chatterbox on the phone; `DirectML.dll` not staged; AVX baseline CI-only; signtool password on the command line; `SECURITY.md` silent on the updater | see the full audit §6 |

## 🧾 Today's checks

| Check | Result |
|---|---|
| fmt | ❌ **10 files would change** (0 on 28 Sep): `studio.rs`, `concat-export/lib.rs`, `dirs.rs`, `titles.rs`, `encode.rs`, `concat-project/{lib,commands/mod,commands/clips}.rs`, `gpu.rs`, `concat-text/lib.rs` |
| clippy | ✅ 0 warnings |
| tests | ⚠️ **672 passed, 1 failed**, 1 ignored (674 in all; 650 on 28 Sep). The failure is `panes::speech::tests::a_voice_name_reads_as_a_person_would_say_it`, the locale race of the 28th, unfixed. E2E 11/11 in 31 s, parity 88/88 in 55 s. |
| perf --check | ✅ 22 of 22 within budget. Model and GPU rows flat or better (composite 1.9 ms, blur 6.3 / 9.8 ms); every decode and export row 20–50 % below the 28th (software 1080p h264 809 fps vs 1 626; 720p export 157 fps vs 204; HDR export 82 vs 111) on a decode path that did not change, so most likely the machine: re-run on a quiet one before reading it as a trend. |
| CI on `main` | ❌ **0 green runs since v0.2.5** (`b78ee56`, 28 Sep 14:19 UTC): 11 failed at fmt, 29 cancelled by the next push. Nix (build only) green 2 Oct. A release tag today would fail at CI and ship nothing. |

## 🗺️ Do next, in order

1. 🟠 **Go green** — `cargo fmt`, push, watch; put the toolchain pin into CI; isolate the two locale tests; write the Unreleased section; bump to 0.2.6.
2. 🟠 **Fence `[[replaces]]`** — refuse one naming a live package at install; carry keys; tell the person when `upgrade_links` changed a document; trial at `params_at(Max)` on a `sibling()` compositor.
3. 🟠 **Stream start in playback audio** — `AudioDecoder::open` takes `start_of`, seeks and trims at `start + offset`; an MTS fixture with sound.
4. 🟡 **Shapes across versions** — bump `DOCUMENT_VERSION`; keep an unknown kind in `extra`; a `shape` on `ClipPatch`; all 44 commands in the round-trip test.
5. 🟡 **Say it in the person's language** — `locales.py --check` flags English values; the six literals and the category bar through `I18n`; the export rate from the timeline; effect keys frozen at the hold's head.
6. 🟡 **The window's seams** — flush before `clip_volume`; epoch on art, strips and save; clear art tables at close; lanes-only publish for monitor frames; `FocusScope` on `Button`, Escape on `Modal`.
7. 🟠 **Then the 28 Sep list, in its order** — sign the update; the Remote page's root, seats and fd leak; processed media out of `cache/`; the migration notices; the export pipeline; the API verbs.

Then HDR phase 4 (a true HDR preview) and phase 5 (zero-copy, Mac hardware encoders).

## ✅ Keep

Layering · `recovered` with no partial file · `hold` as one flag in two places · shapes tidied in the one right place · the scheduler's `Finished` guard · `hardware_wins` with its numbers written down · 37 `unsafe`, 37 SAFETY lines · `wayland_drop.rs` as a careful guest · one epoch gate with a test · dialogs deferred to the next loop turn · monitor request coalescing · 344 probes in half floats · logs with a level of their own · faces mapped once · `.part` → staging → rename · `token::matches` · `locales.py` building `en.json` from the source · the release gate order · parity on lavapipe.
