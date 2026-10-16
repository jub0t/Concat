# Concat audit, 15 October 2026: the quick version

The one-screen scorecard for [`AUDIT-2026-10-15.md`](AUDIT-2026-10-15.md).
Code at `ab21457` (export at `11e1bd3`), 36 commits past the 12 October
audit, with v0.2.7 released in between. The reviewers' own reports are in
[`2026-10-15/`](2026-10-15/).

## In one paragraph

0.2.7 is the first release through every desktop launch check, and about
45 000 people downloaded it in two days. The new code is mostly careful and
tested. But `main` has been red for two days over one stray file, so Linux
has run no tests since 14 October and six PRs merged on top. Android, the
second-largest audience, has never been started by CI, decodes everything
on the CPU, and probably closes on a dark-mode switch. iOS ships without
Import. Windows shipped unsigned. The Intel Mac build does not start on
macOS 12. Phones have not been built since the release.

## Scores

| Aspect | 12 Oct | Today |
|---|:-:|:-:|
| Code quality | 6 | 6 |
| Design architecture | 8 | 7 ↓ |
| Design philosophy | 6 | 5 ↓ |
| Maintainability | 5 | 5 |
| User likeability | 7 | 6 ↓ |
| Performance | 6 | 5 ↓ |
| Scalability | 5 | 5 |
| Security | 6 | 5 ↓ |
| Testing | 6 | 5 ↓ |
| Documentation | 5 | 6 ↑ |
| CI and release | 6 | 4 ↓ |

## The numbers

| | Today | Δ |
|---|---:|---:|
| Rust lines (`src/` only) | 93 423 | +3 061 |
| Tests | 778 | +30 |
| `unsafe` | 110 | +40 (crash handler 32, machine info 6, scale 2) |
| Non-test `unwrap`/`expect` | 68 | +2 |
| `studio.rs` | 11 161 lines | +295 |
| `start()` | 1 870 lines | +98 |
| `cargo test --workspace` | 773 passed, 1 failed (`vn.json`), 1 ignored | |
| fmt, clippy (here) | pass | |
| `locales.py --check` | exit 1 (`vn.json`) | |
| CI on `main` | red since 14 Oct 05:05; last green `e525d7c` | |
| Open issues / PRs | 106 / 6 | +24 / −13 |

## Carried findings

1 fixed (release notes' previous tag) plus the four uncommitted-work asks,
about 5 partly, about 7 worse, the rest unchanged. None of the 11 October
server items has landed.

## New, High

| # | What | Where |
|---|---|---|
| H1 | `main` red two days; Linux ran no tests, clippy or effect GPU render; six PRs merged on red | `locales/vn.json`, `ci.yml:141` |
| H2 | Windows 0.2.7 unsigned; signing step exits 0 without a certificate | `build-app.yml:946-949` |
| H3 | iOS ships without Import; Files app can't see the folder either | `platform.rs:675-694`, `ios-app.sh` |
| H4 | Android launch check never runs the app, reports success | `mobile.yml:301`, `smoke-android.sh:241-277` |
| H5 | Android probably closes on dark mode, split screen, font or locale change (suspected) | `concat-android/Cargo.toml:66`, `lib.rs:388-391` |
| H6 | Intel Mac 0.2.7 does not start on macOS 12.0-13.2 (#333) | `build-app.yml:102, 294-302` |
| N1 | Android never decodes on MediaCodec; every clip CPU-decoded | `hardware.rs:305-306`, `decode.rs:741-743` |
| N2 | A chain of transitions climbs into the next track and hides it | `concat-export/src/lib.rs:806, 816` |
| N3 | Importing a large HDR clip opens NVENC in the background | `proxy.rs:224-243`, `encode.rs:117-130` |
| N4 | No phone built since v0.2.7 | `mobile.yml:53-77` |

## New, Medium

M1 interface scale replaces the display's scale (smaller UI on most
laptops) · M2 export sheet shows a codec it won't use · M3 phone dialogs
and toasts under the system bars · M4 Duplicate is two undo steps and
leaves the whole file · M5 Merge re-encodes instead of a lossless join ·
M6 Remote API fd exhaustion without a token · M7 audio mix holds the
timeline's sound, Cancel ignored · M8 transition sound crossfades only in
the file · M9 FFmpeg's error lines dropped · M10 Android import crash after
process kill · M11 Android projects deleted on uninstall · M12 encoder
probes on the UI thread at start · M13 crash handler takes locks · M14 a
second GPU device per export on Vulkan and Android · M15 0.2.7 notes claim
a reverted fix · M16 README promises gRPC and a CLI no download ships.

## Do first

1. Delete `vn.json`; guard the consolidation test; stop merging on red.
2. Pin `ubuntu-24.04` before 19 October.
3. Android `config_changes` and a guarded second `android_main`.
4. Fail a release without a Windows certificate; get one.
5. A real Android launch check (x86_64 APK or device farm).
6. Interface scale multiplies the display's scale, before it ships.
7. Intel Mac: lazy ONNX Runtime or a 13.3 minimum, plus an `otool` check.
8. Android `*_mediacodec` decoders.
9. iOS document picker and plist keys; README iOS 🚧 until then.
10. Phone workflow on PRs and `main`.
11. Transition lanes from the original track, with a three-clip test.
12. No NVENC first for proxies.
13. Correct the 0.2.7 notes; drop gRPC and the CLI from the README.
