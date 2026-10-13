# Concat audit, 12 Oct 2026 — the quick version

Same audit as `AUDIT-2026-10-12.md`, one screen. Each bar has 10 slots:
🟢 filled = good, ⚪ empty = missing. Severity: 🔴 critical · 🟠 high · 🟡 medium · ⚪ low.
Arrows are against the 11 Oct audit. Every crate was read by one of four reviewers; the headline findings were re-read by hand.

## 🎯 Verdict

**`main` is green; 0.2.7 is not ready to tag.** In the 39 commits since the 11 Oct audit, its top step landed in full:
- **CI names its own hangs.** Per-test timeouts showed which suites hung on Windows, and those suites now skip on the software GPU.
- **Downloads and actions are pinned.** Every desktop download is pinned by digest and every action by SHA.
- **Release bundles must start.** Every desktop bundle is launched where it was built before the release can use it.
- **The window is fixed:** the last three dialogs are off the studio borrow, and ten tracker bugs are fixed, each with a test that fails without its fix.
- **25 languages:** each has a font that covers its script.

**But no release run has passed:**
- **rc.1:** all six desktop builds failed.
- **rc.2:** Linux arm64 crashed on exit. That is fixed in `c06560e`, but no release run has tried the fix yet.
- **The phones' new launch checks have never run.**
- **The final 0.2.7 notes would be three lines long.**

Windows is green because the tests that hit its slow shader compile no longer run there. On an iPhone, Import does nothing.

Count: 🟠 ×4, 🟡 ×17 new, no 🔴.

## 📏 Size

| | Lines | Δ vs 11 Oct |
|---|---:|---:|
| Rust, all crates | 90 362 | +915 (+2 007 like for like; concat-perf removed) |
| Slint UI | 29 901 | +111 |
| WGSL, packages | 1 764 | 0 |
| Tests (`#[test]`) | 748 | +31 like for like |
| Largest file `concat/src/studio.rs` | 10 866 | +177 |

Checks today:
- fmt clean, clippy 0 warnings.
- **745 tests passed, 0 failed, 1 ignored**, on Metal.
- 25 locales, 0 missing.
- 23 of 23 models have their digests.
- CI on `main` green, 6 pushes running.

## 📊 The whole app

| Aspect | Bar | 11 Oct | Today |
|---|---|:-:|:-:|
| 🧹 Code quality | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 |
| 🏛️ Architecture | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 |
| 🧭 Design philosophy | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 |
| 🔧 Maintainability | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5 | 5 |
| 😊 User likeability | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 |
| ⚡ Performance | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 |
| 📈 Scalability | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5 | 5 |
| 🔐 Security | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 5 | 6 ↑ |
| 🧪 Testing | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 |
| 📚 Docs | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5 | 5 |
| 🤖 CI and release | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 4 | 6 ↑ |

- **Security up one:** downloads pinned by digest, actions by SHA, the release tag passed through `env:`, Windows signing by thumbprint.
- **CI up two:** a green gate, hangs that name themselves, and bundles that must start. It is held at 6 because:
  - two release candidates have been spent;
  - phones are never built on a push to main;
  - the bundle workflow can only be run by pushing a tag.

## 🧱 Per crate (average of its scales)

| Crate | Lines | Δ | Bar | Avg | One thing |
|---|---:|---:|---|:-:|---|
| ★ locales + `locales.py` | – | – | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 7.7 | 🟢 25 languages, a glyph test per locale · 🟡 a missing key does not fail CI |
| concat-core | 2 897 | 0 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.4 | 🟢 untouched · ⚪ `MAX_SIDE`'s doc now wrong; two dead items |
| concat-vision | 2 760 | 0 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.3 ↓ | 🟢 untouched · 🟡 CPU-only on shipped Windows; the Intel Mac's runtime may refuse it |
| concat-effects | 7 256 | 0 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.9 ↓ | 🟢 123 packages pass on Metal · 🟠 none compiled on D3D12 in CI any more |
| concat-project | 10 285 | 0 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.9 | ⚪ no commits; `AddClip` places the whole media; tests in one file |
| concat-media | 10 066 | +212 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.8 | 🟢 nothing vendor-specific opened at launch · 🟡 an HEVC export still opens NVENC first (#309 moved) |
| concat-render | 7 723 | +273 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.8 ↑ | 🟢 bounded waits, error scopes, phone limits · 🟡 "30 s" is 2.15 s on GLES; `combine` unscoped |
| concat-server | 1 393 | 0 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.6 | 🟡 `stop` on `0.0.0.0` on Windows, as before |
| concat-text | 2 607 | 0 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.6 | ⚪ no commits |
| concat-speech | 3 307 | +198 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.5 | 🟢 a typed error and one CPU retry, tested · 🟡 Windows ships no DirectML, the crate still asks for it |
| concat (Slint) | 29 901 | +111 | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.5 ↓ | 🟢 a dropdown scrolls under a finger · 🟡 three primitives still break their bindings; the captions sheet re-broke |
| concat-export | 4 039 | +168 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.3 ↑ | 🟢 device loss tested end to end · 🟡 a validation error reads as a lost GPU |
| concat-api | 2 352 | 0 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.1 | 🟡 captions from a clip has no verb either; the `gpu` capability can never appear |
| concat-android | 398 | 0 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.1 | ⚪ 0 tests; the emulator check is uncommitted |
| concat-cli | 514 | 0 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.0 | ⚪ no logger, no signals |
| concat-host | 10 842 | +202 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 5.8 | 🟢 titles painted once; iPhone directories · 🟡 24 held stills undo it mid-drag; saves share a scratch file |
| concat (Rust) | 23 923 | +954 | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 5.6 ↑ | 🟢 every dialog through one seam; 14 tests on pure plans · 🟠 iPhone Import does nothing |

## 🎛️ Per feature (rows that moved, and the weakest)

| Feature | Bar | 11 Oct | Today | State |
|---|---|:-:|:-:|---|
| Languages | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 7 | 8 ↑ | ✅ 25, with faces and READMEs; no right-to-left layout |
| Export | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ⚠️ a stuck GPU ends the export now; a Windows blur or glow can still sit for minutes, uncancellable |
| Timeline editing | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ a click no longer moves a clip; Detach says why |
| Titles and text | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ⚠️ painted once; past 24, they drop out of the monitor during a drag |
| Auto-captions | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 8 | 7 ↓ | ⚠️ ★ on the clip's menu; the sheet can transcribe a clip it is not showing |
| ★ Command-line launch | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | – | 7 | ✅ `--project`, `--version`, `--smoke`; any other argument refuses to start |
| Phone shell | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 7 | 6 ↓ | ❌ iPhone Import does nothing; a failed start vanishes |
| Cutout | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 7 | 6 ↓ | ⚠️ CPU-only on Windows; at risk on Intel Mac |
| Export on a phone | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 | ⚠️ phone limits right; #286 fix uncommitted; unseen on a phone |
| Hardware encode | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 | ⚠️ #309 moved from launch to the first HEVC export |
| ★ Bundle launch checks | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | – | 6 | ⚠️ well built; no target through every check yet |
| Enhance | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 6 | 5 ↓ | ⚠️ CPU on Windows; copies in `cache/` |
| Remote API | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5 | 5 | ⚠️ |
| Self-update | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5 | 5 | ⚠️ no signature check |
| Windows text | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5 | 5 | ⚠️ unseen on Windows |
| Accessibility | 🟢🟢⚪⚪⚪⚪⚪⚪⚪⚪ | 2 | 2 | ❌ |

Every other feature is unchanged; the full table is in the audit, section 4.

## 🚨 Findings

🟠 **High**
1. **The final 0.2.7 notes would be three lines long.** `release.yml:170` takes `v0.2.7-rc.2` as the previous tag, and the CHANGELOG is gone. Adding `--exclude 'v*-*'` gives `v0.2.6`.
2. **On an iPhone, Import does nothing and says nothing.** Only Android installs a file picker (`platform.rs:689-691`).
3. **Phones are never built on a push to `main`.** The new phone checks have never run, and once committed they gate `publish`.
4. **A Windows blur or glow export can sit for minutes compiling shaders (#297).**
   - Nothing bounds or cancels the compile.
   - The shaders are compiled again on every export.
   - CI no longer runs the suites that showed it; they skip and report PASS.

🟡 **Medium**
- **Titles and saves:**
  - Past 24 titles, the early ones drop out of the monitor during a drag, and every title is re-treated each frame (5).
  - Two saves at once share one scratch file (6).
- **Phones:**
  - On GLES the "30 s" device wait is about 2.15 s, so a heavy phone frame fails the export (7).
  - A phone that cannot start just vanishes; an iPhone's Documents folder is hidden from the Files app (12).
- **The captions sheet** can transcribe a clip other than the one it shows (8).
- **GPU errors and encoders:**
  - An HEVC or AV1 export still opens NVENC first, so #309 moved from launch to export (9).
  - Any validation error reads as "lost the GPU 3 times" (10).
  - Transitions' `combine` is unscoped and a refused effect is silently missing from the export (11).
- **Locales:** `locales.py --check` passes with keys missing (13).
- **Release notes:** they carry CI-internal lines and the DXC pair, one of which reverts the other (14).
- **ONNX Runtime:**
  - Windows ships the CPU ONNX Runtime while speech asks for, logs and retries DirectML (15).
  - On an Intel Mac, ONNX Runtime 1.23.2 is likely below ort's API (16).
- **Launch checks:**
  - `--smoke` stops before the window exists (17).
  - The instruction-set limits are still provisional, and the emulated old CPU runs `--version` only (18).
- **Release tooling:**
  - The bundle workflow can only be run by pushing a tag (19).
  - `CODE_SIGNING.md` contradicts the workflow (20).
  - Dependabot's cargo group is red by design over sherpa (21).

⚪ **Low**
- **Phone and launcher:**
  - The 4 px drag threshold is below a finger's slop (22).
  - #151 comes back after Browse (23).
- **Timeline and launch:**
  - Detach unmutes a video the user muted on purpose (24).
  - Any unknown argument refuses to start, so Linux "Open with" through `%U` shows an error box (25).
- **Process and CI:**
  - Ten "Fixes #N" commits; the push closed #299, which was reopened by hand (26).
  - The concurrency group cancels pending runs on `main`, and 7 pushes have no CI (28).
  - `ubuntu-latest` becomes Ubuntu 26 on 19 Oct (29).
- **Rendering:**
  - Right-to-left languages are laid out left to right (27).
  - The live-title ceiling is fixed at 512 MiB rather than sized to the machine (30).
  - The 8-bit fallback clips HDR (31).
- **API, server and CLI:** the `gpu` capability can never appear; a gRPC stream can hold `stop`; CLI `render` writes in place (32).
- **Docs:** three misattached doc comments, the DirectML docs, the README badge and MCP, `ARCHITECTURE.md`'s known issues (33).
- **PR hygiene:** #304 is still open; #314 needs a re-run; 11 contributor PRs are unreviewed (34).

## 🧭 What the 11 Oct audit asked, and what landed

✅ **Landed:**
- Windows hangs named, GPU suites skipped on WARP, engine job timed out, toolchain action pinned (no tag yet).
- The three dialogs.
- Digests, SHA pins, `env:` for the tag, signing secrets declared, the real Android key used twice.
- The locale check prints anywhere and counts English.

🟡 **Partly:** uncaptured errors are counted and scoped, with half of it left.

❌ **Not landed:**
- Relink, cache clear and trials off the UI thread.
- The paste.
- `Server::stop` and `catch_unwind`.
- The pinned-frame budget and its kin.
- The literals through `I18n`.
- Anything seen on real Windows or a real phone.

## ▶️ Next, in order

1. **Release:**
   - Fix the previous-tag lookup.
   - Finish and commit the uncommitted phone work, on its own:
     - the doc line;
     - a patient final drain;
     - `smoke-ios.sh` checks the exit status;
     - a `trap` in `smoke-android.sh`;
     - "Refs #286".
   - Run `mobile.yml` by hand.
   - Tag rc.3 and let every target finish.
   - Tag 0.2.7, with the draft's notes edited by hand.
2. An iPhone document picker (or a notice); a failed start shown on a phone; the two plist keys.
3. Make `Select`, `SegmentedControl` and `Switch` controlled.
4. Held stills stamped per pass; one scratch name per save.
5. Windows shaders: shared pipelines, cancel between builds, a visible skip, a scheduled real-GPU job, then DXC again on real hardware.
6. GPU errors: refused vs lost, `combine` scoped, patient GLES waits; a sentinel and a setting for vendor encoders.
7. CI:
   - `push` on `mobile.yml`, `workflow_dispatch` on `build-app.yml`;
   - `--smoke` through real frames, instruction-set limits from rc.2's counts;
   - pin `ubuntu-24.04` before 19 Oct;
   - Dependabot's sherpa exclusion;
   - `locales.py` fails on missing keys.
8. Honest ONNX providers on Windows and Intel Mac; `CODE_SIGNING.md` in line with the workflow.
9. Then the 11 Oct steps still open: the paste, the server, the pinned-frame budget, and the 28 Sept list.
