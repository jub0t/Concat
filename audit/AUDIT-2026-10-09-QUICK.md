# Concat audit, 9 Oct 2026 — the quick version

Same audit as `AUDIT-2026-10-09.md`, one screen. Each bar has 10 slots:
🟢 filled = good, ⚪ empty = missing. Severity: 🔴 critical · 🟠 high · 🟡 medium · ⚪ low.
Arrows are against the 4 Oct audit. Every crate was read in full by one of four reviewers.

## 🎯 Verdict

**A good week for the code, a bad week for the pipeline.** Five days and 54 commits after the last audit, two of its three headline findings are closed (`[[replaces]]` fenced with tests, the preview's sound at the stream start), fmt is green, the window suite no longer races, and the day's six fixes each land with a test that fails without them: phones export and can find the file, stills keep their shape, Detach means detached, Chinese and Korean have a face to fall back on. But the release pipeline has three holes it did not have on the 4th's radar: **0.2.6 was built from `main`, 35 commits past its tag**; `main` has been red for five days on a Windows test that hangs; and the Android release key is readable by every pull-request build. In the engine, an export with sound can leave a broken file at the final path. 🟠 ×5 new, no 🔴.

## 📏 Size

| | Lines | Δ vs 4 Oct |
|---|---:|---:|
| Rust, all crates | 88 879 | +1 503 |
| Slint UI | 29 717 | +921 |
| WGSL, packages | 1 764 | 0 |
| Tests (`#[test]`) | 713 | +37 |
| Largest file `concat/src/studio.rs` | 10 435 | +406 |

Checks today: fmt clean · clippy 0 warnings · 711 tests pass, 1 ignored · locales 0 missing · 22/22 perf scenarios in budget (on a loaded machine) · CI on `main` red.

## 📊 The whole app

| Aspect | Bar | 4 Oct | Today |
|---|---|:-:|:-:|
| 🧹 Code quality | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 |
| 🏛️ Architecture | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 |
| 🧭 Design philosophy | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 |
| 🔧 Maintainability | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5 | 5 |
| 😊 User likeability | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 |
| ⚡ Performance | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 |
| 📈 Scalability | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5 | 5 |
| 🔐 Security | 🟢🟢🟢🟢⚪⚪⚪⚪⚪⚪ | 4 | 4 |
| 🧪 Testing | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 |
| 📚 Docs | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 |
| 🤖 CI and release | 🟢🟢🟢⚪⚪⚪⚪⚪⚪⚪ | 3 | 3 |

Flat across the board: what landed was good and what it cost was elsewhere. Security stays at 4 because the hijack closed and the model downloads are pinned, while the release key, the untagged release and the unverified FFmpeg opened.

## 🧱 Per crate (average of its scales)

| Crate | Lines | Δ | Bar | Avg | One thing |
|---|---:|---:|---|:-:|---|
| concat-vision | 2 760 | 0 | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 7.5 ↑ | 🟢 untouched, pinned, digested · ⚪ runtime wrapper untested |
| concat-core | 2 891 | +37 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.3 | 🟢 `FrameRate::checked` in lowest terms · ⚪ `reduce` panics on file rates |
| concat-effects | 7 256 | +160 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.3 | 🟢 every take-over refused with a reason · 🟡 trials on the UI thread, a catalogue leaked per reload |
| concat-project | 10 216 | +751 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.3 ↑ | 🟢 placement is a model rule, 10 tests · 🟡 a 1e300 start panics the export |
| concat-server | 1 393 | +17 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.6 ↓ | 🟢 Hub panics are one caller's error · 🟠 fd leak, pre-auth seats, a reader that stops pins its seat |
| concat-speech | 3 109 | 0 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.5 | 🟢 downloads pinned to commits · ⚪ no change, stale docs |
| concat-media | 9 832 | +338 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.6 ↓ | 🟢 stream start fixed with a fixture; 38 `unsafe`, 38 SAFETY · 🟠 the mux writes the final path |
| concat-text | 2 607 | +633 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.6 ↓ | 🟢 colour emoji probed, not trusted · 🟡 12 MB copied to the heap, PNG decoded per render |
| concat-api | 2 351 | 0 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.4 ↓ | 🟢 confined writes · 🟡 reads unconfined, nine window operations it cannot name |
| concat-render | 7 432 | −692 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.5 | 🟢 scopes gone cleanly · 🟡 no uncaptured-error handler, a 10 000-px frame panics |
| concat-android | 398 | +63 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.0 | 🟢 publisher reports `[path, why]` · 🟡 a provider exception kills the process · ⚪ 0 tests |
| concat-cli | 514 | 0 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.0 ↓ | ⚪ no logger, no signal handling, token on argv |
| concat-host | 10 630 | −1 239 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 5.8 | 🟢 `catch_unwind`, memory-sized caches, voiceover · 🟡 `Server::stop` blocks the UI; crashed takes lost |
| concat-export | 3 844 | +142 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 5.9 ↓ | 🟢 folder and still fixed with tests · 🟠 mux not atomic; silent frame loss on one path |
| concat (Slint) | 29 717 | +921 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.5 | 🟢 Escape on the confirm sheet · 🟡 2 `accessible-*` in 29 717 lines |
| concat (Rust) | 22 554 | +1 293 | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5.1 ↓ | 🟢 three `expect`s in 22.5k lines · 🟡 dialogs under a `RefCell` borrow; `studio.rs` +406; 71 tests |

## 🎛️ Per feature

| Feature | Bar | 4 Oct | Today | State |
|---|---|:-:|:-:|---|
| Launcher | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ |
| Import and bin | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ ★ click to preview |
| Timeline editing | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ ★ never lands on top, ★ detach alone |
| Titles and text | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 7 | 8 ↑ | ✅ ★ colour emoji, ★ font picker |
| Auto-captions | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 7 | 8 ↑ | ✅ words at the right time on MTS |
| Effects and filters | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ `[[replaces]]` fenced |
| Undo / redo | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ |
| LUTs | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ |
| Relink | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 8 | 6 ↓ | ⚠️ "Relink all" walks on the UI thread, follows symlink loops |
| Themes | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ |
| Playback and audio | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 5 | 7 ↑ | ✅ stream start, one-word clock, window decoded ahead |
| Transitions | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ⚠️ hold plays keyed effects early |
| Colour and HDR | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ AMF Main 10 |
| Colour grading | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Keyframes | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Crop, blend, masks | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ shape masks still absent (#239) |
| Cutout | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Text-to-speech | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Templates | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Custom packages | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6 | 7 ↑ | ✅ trials at Default, Min, Max on a sibling |
| Export | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ⚠️ ★ makes its folder; mux not atomic; overwrites without asking |
| ★ Monitor stills | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | – | 7 | ✅ keeps its shape, pixel test |
| ★ Export on a phone | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | – | 6 | ⚠️ into Movies/Concat; unseen on a phone |
| Phone shell | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6 | 7 ↑ | ✅ exports, findable, stills right |
| Languages | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6 | 7 ↑ | ✅ ★ CJK face held to the locales; ja real; de/fr/ko/pt-BR half English |
| Volume line | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6 | 7 ↑ | ✅ flushes the title commit |
| ★ Voiceover | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | – | 6 | ⚠️ a crashed take is lost |
| Hardware encode | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 | ✅ |
| Enhance | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 | ⚠️ copies in `cache/` |
| Shapes / Stickers | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 | ⚠️ vanish under 0.2.5 |
| Keyboard menus | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 | ⚠️ |
| Windows data folders | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 | ✅ |
| Remote API | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 6 | 5 ↓ | ⚠️ nine window operations it cannot name; `stop` blocks the UI |
| Proxies | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5 | 5 | ⚠️ |
| Self-update | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5 | 5 | ⚠️ no signature |
| ★ VU meters | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | – | 5 | ⚠️ programme meter has no feed |
| ★ Windows text | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | – | 5 | ⚠️ pixel-snapped at 1x; unseen on Windows |
| Accessibility | 🟢🟢⚪⚪⚪⚪⚪⚪⚪⚪ | 2 | 2 | ❌ |
| Scopes | – | 7 | – | removed |

## 🚨 Findings

🟠 **High**
1. 0.2.6 was built from `main`, 35 commits past its tag: the build workflows check out with no `ref` (`build-app.yml:152`, `mobile.yml:58`).
2. `main` red five days: the Windows test job hangs in `cards::tests::every_card_draws…` and dies at 90 min; the release gate reuses it.
3. The Android release key and password reach pull-request builds and every build script (`mobile.yml:36, 113-130`).
4. An export with sound muxes straight into the final path; a failure leaves a truncated file where the last export was (`audio.rs:720`, `concat-export/src/lib.rs:992`).
5. The CJK face shipped with half of Big5 and no JIS; its test proved registration, not coverage. Fixed at the head with a coverage test.

🟡 **Medium** — dialogs open under the studio's `RefCell` borrow (6) · the export sheet overwrites without asking and accepts `/` in a name (7) · a frame over 8192 px panics wgpu (8) · a 1e300 clip start panics the export (9) · a SAFETY line on `set_var` is untrue (10) · a group dragged past 0 collapses (11) · a speed curve grows over a neighbour (12) · trials on the UI thread, a catalogue leaked per reload (13) · "Relink all" on the UI thread (14) · fifteen English strings in Rust and Slint (15) · `Server::stop` blocks the window; jobs and decode workers die silently on a panic (16) · API reads unconfined, caller strings to FFmpeg's opener (17) · one export path drops failed frames silently (18) · pinned prefetch frames outside any budget (19) · a crashed voiceover take is lost (20) · unverified FFmpeg and sherpa downloads, mutable action tags (21) · three Android bridge crash paths (22).

⚪ **Low** — doc drift in gpu.rs, studio.rs, pool.rs, ARCHITECTURE (23) · tests that skip in CI or prove the wrong thing (24) · Rational edges, negative starts, RIFF overflow, `DefaultHasher` proxy names (25) · non-ABI side-data sizes (26) · realtime audio frees and faults (27) · blocking IO on the UI thread (28) · hygiene (29).

## 🧭 What the 4 Oct audit asked, and what landed

✅ fmt, locale lock, 0.2.6 section · ✅ `[[replaces]]` fence, keys, notice, trials on a sibling · ✅ stream start with a fixture · ✅ export rate, volume flush, art caches · ❌ document version for shapes · ❌ `FocusScope`/Escape/`accessible-label` · ❌ the 28 Sept list (update signing, server seats and fds, API reach).

## ▶️ Next, in order

1. `ref:` the tag in both build workflows; Android signing on release runs only; skip or time out the cards test without a GPU; toolchain into CI; tag 0.2.7 from a green `main`.
2. Mux into scratch and rename; one failure policy for the export; a size ceiling with an uncaptured-error handler; bound clip times and probed rates.
3. Export sheet: refuse `/` and empty names, ask before overwriting; dialogs on the next loop turn.
4. Trials, relink, cache clear and sweep off the UI thread.
5. `Server::stop` cancels first; `catch_unwind` on jobs and workers; a protocol whitelist before FFmpeg; a write timeout.
6. Pinned-frame budget; WAV placeholder header; group clamp; speed-curve ripple.
7. The literals through `I18n`; a locale check that flags English.
8. Digests for downloads; actions by SHA; `inputs.tag` through `env:`.
9. See Windows text and the phone's export on real hardware. Then the 28 Sept list.
