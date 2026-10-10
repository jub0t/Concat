# Concat audit, 11 Oct 2026 — the quick version

Same audit as `AUDIT-2026-10-11.md`, one screen. Each bar has 10 slots:
🟢 filled = good, ⚪ empty = missing. Severity: 🔴 critical · 🟠 high · 🟡 medium · ⚪ low.
Arrows are against the 9 Oct audit. Every crate was read in full by one of four reviewers.

## 🎯 Verdict

**Three of nine steps in two days, and the gate is still shut.** Eight commits after the 9 October audit, its three headline holes are closed and closed well: a release builds its tag and the Android key reaches the signing step alone (3f3b7de); an export can no longer leave a broken file, and no document can panic the GPU (a2dd5e3); the export sheet asks before replacing and refuses a name that is not a file's (9a2e5f2). Copy and paste works across timelines as one undo step. Each fix carries a test that fails without it. But `main` has now been red for a week: the Windows job still times out at 90 minutes **after** the fix meant to free it, nobody can read why, and no tag can ship. Three of the eight native dialogs still open under the studio borrow, a paste onto a busy lane lands in the wrong place with the wrong frames, and the toolchain action now runs from a moving branch. 🟠 ×1, 🟡 ×9 new, no 🔴.

## 📏 Size

| | Lines | Δ vs 9 Oct |
|---|---:|---:|
| Rust, all crates | 89 447 | +568 |
| Slint UI | 29 790 | +73 |
| WGSL, packages | 1 764 | 0 |
| Tests (`#[test]`) | 719 | +4 like for like |
| Largest file `concat/src/studio.rs` | 10 689 | +254 |

Checks today: fmt clean · clippy 0 warnings · tests and perf see the full audit · locales 0 missing (the script crashes on a Windows console) · 23/23 models digested · CI on `main` red, 7 days.

## 📊 The whole app

| Aspect | Bar | 9 Oct | Today |
|---|---|:-:|:-:|
| 🧹 Code quality | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 |
| 🏛️ Architecture | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 |
| 🧭 Design philosophy | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 |
| 🔧 Maintainability | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5 | 5 |
| 😊 User likeability | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 |
| ⚡ Performance | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 |
| 📈 Scalability | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5 | 5 |
| 🔐 Security | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 4 | 5 ↑ |
| 🧪 Testing | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 |
| 📚 Docs | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 6 | 5 ↓ |
| 🤖 CI and release | 🟢🟢🟢🟢⚪⚪⚪⚪⚪⚪ | 3 | 4 ↑ |

Security up one: the key, the tag, the ceilings, the atomic export. Docs down one: the CHANGELOG is 45 commits behind and `ARCHITECTURE.md` had to be rewritten. CI up one for a release workflow that is now right, held at 4 by a gate that cannot pass.

## 🧱 Per crate (average of its scales)

| Crate | Lines | Δ | Bar | Avg | One thing |
|---|---:|---:|---|:-:|---|
| concat-vision | 2 760 | 0 | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 7.5 | 🟢 untouched since 23 Sept · ⚪ runtime wrapper untested |
| concat-core | 2 897 | +6 | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 7.4 ↑ | 🟢 `MAX_SIDE` is the one ceiling everyone reads · ⚪ two dead items, `Neg` on `i64::MIN` |
| concat-effects | 7 256 | 0 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.3 | 🟢 no churn · 🟡 trials on the UI thread, a catalogue leaked per reload |
| concat-project | 10 285 | +69 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.1 ↓ | 🟢 every clip time held to 100 h, with a test · ⚪ all 114 tests in one 4 258-line module |
| concat-media | 9 854 | +22 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.8 ↑ | 🟢 absurd rates refused at the probe; 38 `unsafe`, 38 SAFETY · ⚪ a VFR average past 1e6 now fails the import |
| concat-render | 7 450 | +18 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.6 ↑ | 🟢 an uncaptured error logs, nothing dies · 🟡 but the frame it leaves is written unmarked |
| concat-server | 1 393 | 0 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.6 | 🟢 Hub panics are one caller's error · 🟡 `stop` can hang for good on `0.0.0.0` on Windows; a refused thread kills the accept loop |
| concat-text | 2 607 | 0 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.6 | 🟢 colour emoji probed, not trusted · ⚪ 12 MB to the heap, PNG per render |
| concat-speech | 3 109 | 0 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.5 | 🟢 downloads pinned to commits · ⚪ the DirectML failure of #291 is a bare error |
| concat-export | 3 871 | +27 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.1 ↑ | 🟢 one rename, one failure policy · ⚪ no test for the failure paths; scratch names collide |
| concat-api | 2 352 | +1 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.1 ↓ | 🟢 `MAX_SIDE` is the engine's · 🟡 two more window operations with no verb; `export.run` overwrites silently |
| concat-android | 398 | 0 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.1 ↑ | 🟢 the release key stays out of pull requests · ⚪ 0 tests, three crash paths |
| concat (Slint) | 29 790 | +73 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.7 ↑ | 🟢 the tray's nested popup is gone · 🟡 2 `accessible-*`, no keyboard into the new menu |
| concat-cli | 514 | 0 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.0 | ⚪ no logger, no signals, token on argv |
| concat-host | 10 640 | +10 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 5.8 | 🟢 the cards test skips on WARP by reading the adapter · 🟡 the update chain and `stop` on the UI thread as they were |
| concat (Rust) | 22 969 | +415 | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5.4 ↑ | 🟢 `after_dialog` is the right seam; the paste plan is pure and tested · 🟡 three dialogs missed; the paste lands wrong on a busy lane; `studio.rs` +254 |

## 🎛️ Per feature

| Feature | Bar | 9 Oct | Today | State |
|---|---|:-:|:-:|---|
| Launcher | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ Browse off the borrow; Open still on it |
| Import and bin | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ |
| Timeline editing | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ ★ copy and paste across timelines, one undo step |
| ★ Copy and paste | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | – | 6 | ⚠️ the plan is right; the landing is not on a busy lane; a title keeps a quarter |
| Export | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 7 | 8 ↑ | ✅ ★ atomic with sound; ★ asks before replacing; a GPU error is an unmarked frame |
| Save Frame, Save Audio | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 7 | 8 ↑ | ✅ dialogs on their own turn of the loop |
| Titles and text | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ |
| Auto-captions | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ |
| Effects and filters | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ |
| Undo / redo | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ |
| LUTs | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ |
| Themes | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ |
| Playback and audio | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Transitions | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ⚠️ the hold plays keyed effects early |
| Colour and HDR | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Colour grading | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Keyframes | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Crop, blend, masks | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ shape masks still absent (#239) |
| Cutout | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Text-to-speech | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ Chatterbox on DirectML fails bare (#291) |
| Templates | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Custom packages | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ⚠️ trials on the UI thread |
| Monitor stills | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Hardware decode | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Phone shell | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ⚠️ unseen on a phone; iOS cannot create a project (#299) |
| Languages | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ the week's keys in all 14; seven locales half English by value |
| Volume line | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ untested |
| Logs | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| VU meters | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 5 | 6 ↑ | ⚠️ ★ a ruler, a peak readout, fed from playback; no test |
| Voiceover | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 | ⚠️ a crashed take is lost |
| Export on a phone | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 | ⚠️ unseen on a phone |
| Relink | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 | ⚠️ picker off the borrow; the walk still on the UI thread |
| Hardware encode | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 | ✅ |
| Enhance | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 | ⚠️ copies in `cache/` |
| Shapes / Stickers | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 | ⚠️ vanish under 0.2.5; a pasted shape loses its transform |
| Keyboard menus | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 | ⚠️ |
| Windows data folders | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 | ✅ |
| Remote API | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5 | 5 | ⚠️ seven operations with no verb; `stop` blocks, and may hang on Windows |
| Proxies | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5 | 5 | ⚠️ |
| Self-update | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5 | 5 | ⚠️ no signature |
| Windows text | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5 | 5 | ⚠️ unseen on Windows |
| Accessibility | 🟢🟢⚪⚪⚪⚪⚪⚪⚪⚪ | 2 | 2 | ❌ |

## 🚨 Findings

🟠 **High**
1. `main` is still red after the fix: the Windows job times out at 90 min in `cargo test` on both runs since 3f3b7de, the log is admin-only, the cards test should skip, so something else hangs. No tag can ship (`ci.yml:272`, `release.yml:67-72`).

🟡 **Medium** — three dialogs still open under the studio borrow: Open, Import LUT, the menu's Import (2) · a paste lands wrong on a busy lane (the whole media is placed before the trim) and a pasted title keeps style, length and height only (3) · `dtolnay/rust-toolchain@master` in six jobs (4) · CHANGELOG 45 commits behind (5) · `Server::stop` hangs for good on a wildcard bind on Windows (6) · a refused thread kills the accept loop silently (7) · an uncaptured GPU error is an unmarked export frame (8) · `studio.rs` +254, one `impl` of 215 methods on 93 fields (9) · `export.run` and `project.create` write where the window now asks (10).

⚪ **Low** — mux scratch names collide across exporters (11) · past-the-end differs between decode paths (12) · `pick_rate` and VFR averages (13) · the ceilings' five leftovers (14) · gRPC stream open after a drop, a poisoned mutex, a non-UTF-8 CLI line (15) · the cards skip under `CONCAT_REQUIRE_GPU`, no timeout on the engine job (16) · apksigner's alias and password edges, undeclared Windows signing secrets (17) · `refresh_art` skipped, the clipboard outlives the project (18) · concat-project's 4 258-line test module (19) · `locales.py` crashes on cp1252 (20) · five CPU-fallback comments, "Slint 1.17", three crates missing from `src/README.md` (21) · dead `source_duration` and `Project` (22).

## 🧭 What the 9 Oct audit asked, and what landed

✅ `ref:` the tag, Android signed on release runs only, the cards skip, the toolchain pin · ✅ atomic mux, one failure policy, `MAX_SIDE` with handlers, `MAX_TIME` and checked rates · ✅ the export sheet; 5 of 8 dialogs · ❌ trials, relink, cache clear off the UI thread · ❌ `Server::stop`, `catch_unwind`, the whitelist · ❌ pinned-frame budget, WAV header, group clamp, speed-curve ripple · ❌ the literals, the locale check · ❌ digests and SHA pins (one action moved to a branch) · ❌ anything seen on real Windows or a real phone.

## ▶️ Next, in order

1. Make the Windows job name its hang (nextest per-test timeout, or `--test-threads=1 --nocapture`); skip the GPU suites on WARP; pin the toolchain action to a SHA; write the CHANGELOG; tag 0.2.7 from a green `main`.
2. The three dialogs through `after_dialog`; relink, cache clear, sweep and trials off the UI thread.
3. The paste: place at the trimmed length, carry the full patch for titles, clear with the project; move the placement half into `concat-project`.
4. `Server::stop` on loopback and cancelling first; `Builder::spawn`; `catch_unwind` on jobs and workers; a protocol whitelist; a write timeout; `exists` and `is_sane` on the API's writing verbs.
5. Count uncaptured errors and say so; random scratch suffixes; one past-the-end rule; a VFR fixture.
6. Digests and SHA pins; declare the Windows signing secrets; a release against a staging tag with the real key.
7. Pinned-frame budget; WAV header; group clamp; speed-curve ripple; `DOCUMENT_VERSION` for shapes.
8. The literals through `I18n`; a locale check that flags English and prints anywhere.
9. The 28 Sept list; then Windows text and the phone's export on real hardware.
