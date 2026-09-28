# Concat audit, 28 Sep 2026 — the quick version

Same audit as `AUDIT-2026-09-28.md`, one screen. Each bar has 10 slots:
🟢 filled = good, ⚪ empty = missing. Severity: 🔴 critical · 🟠 high · 🟡 medium · ⚪ low.
Arrows are against the 23 Sep audit. *prov.* = that crate's review was still
running when this was written; score carried from 23 Sep and adjusted only by hand checks.

## 🎯 Verdict

**The pipeline is one thing now, and the guarantees are half paid.** Five days and 143 commits after the last audit, the picture path is one GPU compositor in linear light with HDR in and out, 9 of the 24 findings are closed (4 of the 6 criticals), 10 are part-closed, and `main` is green. No new critical. What came in with the speed: a self-update whose trust root is the same GitHub release it downloads from, a Remote API rooted at the home folder, and three seams where pre-0.2.5 documents open changed and say nothing.

## 📊 The whole app

| Aspect | Bar | 23 Sep | Today |
|---|---|:-:|:-:|
| 🧹 Code quality | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 |
| 🏛️ Architecture | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 7 | 8 ↑ |
| 🧭 Design philosophy | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6 | 7 ↑ |
| 🔧 Maintainability | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 |
| 😊 User likeability | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 |
| ⚡ Performance | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 5 | 6 ↑ |
| 📈 Scalability | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 4 | 5 ↑ |
| 🔐 Security | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 4 | 5 ↑ |
| 🧪 Testing | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6 | 7 ↑ |
| 📚 Docs | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 |
| 🤖 CI and release | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 4 | 6 ↑ |

## 🧱 Per crate (average of its six scales)

| Crate | Bar | Avg | One thing |
|---|---|:-:|---|
| concat-effects *prov.* | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 7.7 ↑ | 🟢 format 2, probes on all 98 · 🟡 `[[replaces]]` drops keys |
| concat-core | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.3 | 🟢 no deps, rational time · ⚪ dead `Project`, linear lookup |
| concat-cli | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.3 | 🟢 token from env · ⚪ `serve` parks, socket left |
| concat-api | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.2 | 🟢 real roots check · 🟠 rooted at the home folder |
| concat-server | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.2 ↑ | 🟢 line, seat, outbox caps · 🟠 fd leak per connection |
| concat-vision *prov.* | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.2 | 🟢 unchanged, ORT wrapped · ⚪ serial batch-1 |
| concat-text *prov.* | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.2 | 🟢 any font via fontdb · ⚪ per-render copy to confirm |
| concat-project | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.8 ↑ | 🟢 `clips_where` + one tidy pass · 🟡 no frame ceiling, bin cloned per snapshot |
| concat-media *prov.* | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.8 ↑ | 🟢 `catch_unwind`, `start_of` · ⚪ copies per frame to confirm |
| concat (Slint) *prov.* | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.8 | 🟢 one Modal pattern · ⚪ two `accessible-*` in 27 346 lines |
| concat-render *prov.* | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.7 ↑ | 🟢 one compositor, oracle in tests · ⚪ `gpu.rs` 3 599 lines |
| concat-speech *prov.* | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.2 | 🟢 CoreML with CPU fallback · 🟡 only the window can reach it |
| concat-export | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.0 | 🟢 device lent in · ⚪ serial loop, 40-field `ExportClip` |
| concat-host | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.0 ↑ | 🟢 corrupt doc refused, with tests · 🟠 update trust chain |
| concat (Rust) *prov.* | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.0 | 🟢 project epoch exists · ⚪ `studio.rs` 9 461 lines |

## 🎛️ Per feature

| Feature | Bar | 23 Sep | Today | State |
|---|---|:-:|:-:|---|
| Launcher | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ exact NTSC rates |
| Import and bin | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ |
| Timeline editing | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ trim-follow, Rotate |
| Undo / redo | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ sharing survives ripple |
| Transitions | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 6 | 8 ↑ | ✅ in the monitor, pinned by pixel |
| Effects and filters | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 7 | 8 ↑ | ✅ 98 GPU packages in light |
| LUTs | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 7 | 8 ↑ | ✅ BOM, Resolve range, float |
| Languages | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 6 | 8 ↑ | ✅ 14 locales, all 100 % |
| Relink | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ |
| Themes | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | 8 | ✅ |
| ★ Colour and HDR | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | – | 7 | ✅ HDR in and out; pre-0.2.5 imports keep SDR proxies |
| ★ Scopes | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | – | 7 | ✅ GPU-counted (render review pending) |
| ★ Colour grading | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | – | 7 | ✅ wheels keyable, curves not |
| Custom packages | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 5 | 7 ↑ | ✅ trialled at install, error scopes |
| Crop, blend, masks | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6 | 7 ↑ | ✅ Darken/Lighten fixed |
| Titles | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ any font; missing font silent |
| Keyframes | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ⚠️ old preset animations dropped silently |
| Cutout | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Captions | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Text-to-speech | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ verified downloads |
| Templates | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ |
| Export | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ window's device, CBR, HDR; one thread |
| Hardware decode | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | 7 | ✅ per-stream choice; not zero-copy |
| Enhance | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 | ⚠️ one model; copies in `cache/` |
| Playback and audio | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 | ⚠️ 400 pieces per curved clip |
| Remote API | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | 6 | ⚠️ home root, fd leak, second GPU device |
| ★ Phone shell | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | – | 6 | ⚠️ real shell; no phone build on push |
| Proxies | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 4 | 5 ↑ | ⚠️ swept at open; still no UI |
| ★ Self-update | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | – | 5 | ⚠️ no signature; downgrade one click |
| Accessibility | 🟢🟢⚪⚪⚪⚪⚪⚪⚪⚪ | 2 | 2 | ❌ two `accessible-*` properties |

## 🔁 The 23 Sep findings

| Closed (9) | Partly closed (10) | Open (2) | Pending a review (3) |
|---|---|---|---|
| 1 corrupt doc · 2 scheduler panic · 5 two engines · 6 model digests · 7 hostile packages · 8 undo sharing · 9 start time · 10 Darken/Lighten · 12 CI | 3 wrong-project results · 4 API paths and sizes · 11 art scan · 13 clamps · 14 E2E pixels · 15 cache bounds · 17 Hub thread · 21 UI facts · 23 docs · 24 hygiene | 19 speech only in the window · 22 dead `ui/demo/` | 16 per-frame copies · 18 ORT twice · 20 font copied per render |

## 🚨 Top new findings

| # | Sev | What | Where |
|:-:|:-:|---|---|
| 1 | 🟠 | Self-update: digest published beside the package, no signature, quarantine stripped, Windows installer unsigned | `concat-host/src/updates.rs:45,381,430,539` · `release.yml:100` |
| 2 | 🟠 | Remote page's API is rooted at the home folder: a token holder can truncate `~/.ssh/authorized_keys`; no `HOME` = anywhere; open-then-save escapes | `concat-api/src/lib.rs:648,950` · `concat-server/src/lib.rs:80` |
| 3 | 🟠 | Every JSON connection leaks a file descriptor; seats counted before auth | `concat-server/src/json.rs:106,161,102` |
| 4 | 🟠 | Clear cache deletes the media behind reversed and enhanced clips; never swept | `concat-host/src/reverse.rs:71` · `projects.rs:330` |
| 5 | 🟠 | 0.2.4 documents with reversed clips or preset animations open changed, no notice | `concat-project/src/doc.rs:62` |
| 6 | 🟡 | Embedded API still opens a second GPU device (the #202 path) | `concat-api/src/lib.rs:179,761` |
| 7 | 🟡 | `[[replaces]]` drops keyframes, is silent, skips user packages and templates | `concat-project/src/editor.rs:89` · `catalogue.rs:585` |
| 8 | 🟡 | HDR proxies not regenerated for pre-0.2.5 imports | `concat/src/studio.rs:6123` · `model.rs:171` |
| 9 | 🟡 | Hub: heavy work in line, no `catch_unwind`, slow event reader silently dropped | `concat-server/src/hub.rs:58,83` |
| 10 | 🟡 | Downgrade is one click with no gate | `updates.rs:141` · `settings.rs:447` |
| 11 | 🟡 | No ceiling on frame size or rate in the model | `commands/timelines.rs:54` |
| 12 | 🟡 | Every undo snapshot deep-copies the bin | `concat-project/src/editor.rs:141` |
| 13 | 🟡 | Export still one thread with a readback per frame; 400-piece curves; mixer scans every clip | `concat-export/src/lib.rs:1204,1005` · `playback.rs:1032` |
| 14 | ⚪ | Font by absolute path, missing font silent | `model.rs:1797` · `titles.rs:211` |
| 15 | ⚪ | `has_non_finite` misses 3 fields; no-op undo steps; HDR re-flip; no `extra` on `video`/`text`; `"concat": "0.1.0"` | `commands/mod.rs:878` · `doc.rs:317` |
| 16 | ⚪ | Socket umask mode; `connections()` over-counts; `export.progress.stage` names differ from docs | `concat-server/src/lib.rs:196` · `message.rs:591` |
| 17 | ⚪ | `rpm -q` on the UI thread; predictable export temporaries; two TLS stacks | `settings.rs:796` · `lib.rs:887` |
| 18 | ⚪ | Carried: 5 ⌘ glyphs, `ui/demo/`, dead `core::Project`, `Path::exists` in the wasm crate, speed range ×5 | various |

## 🧾 Today's checks

| Check | Result |
|---|---|
| fmt | ✅ 0 files would change (8 on 23 Sep) |
| clippy | ✅ 0 warnings |
| tests | ⚠️ **647 passed, 2 failed**, 1 ignored (650 in all, 544 on 23 Sep). Both failures are window tests that saw German where they expected English (`format::tests::phrases_are_coarse`: "gestern" for "yesterday"; `panes::speech::tests::a_voice_name_reads_as_a_person_would_say_it`): the process-wide locale leaking between tests in one binary (confirmed: both pass alone, and the suite passes with `--test-threads=1`), not a product bug. CI on `main` is green. |
| perf --check | ✅ 22 of 22 within budget: GPU composite 2.3 ms (3.2), 720p export 204 fps (115), HDR export 111 fps, 4K HLG deep-for-GPU 140 fps vs 29 tone-mapped on the CPU, 4K HEVC 10-bit hardware 112 fps and software 43, three 4K streams 61 fps, 4K Gaussian blur 6.1 ms at radius 10 and 9.3 at 50 |
| CI on `main` | ✅ green at head; 3 of today's 12 runs were red between 10:12 and 11:02 UTC, fixed by `d1d3265` |

## 🗺️ Do next, in order

1. 🟠 **Sign the update** — a key compiled into the app, a signed manifest, keep quarantine, confirm downgrade; sign the Windows installer and notarise every macOS build.
2. 🟠 **Remote page defaults** — root at project and export folders; refuse to start with no root; check `writable` on open or save; seats after auth; drop the connection clone on close; `catch_unwind` round the Hub; lend the window's device to the embedded API.
3. 🟠 **Processed media out of `cache/`** — reverse and enhance copies under `media/`; sweep art and peaks; re-probe pre-0.2.5 media at open.
4. 🟠 **Say it when a document changes** — a migration for `reverse` and the animations; surface `upgrade_links` and carry its keys; run it for templates and user packages.
5. 🟡 **Finish the 23 Sep list** — epoch on every worker path; a tidy for the frame; one speed range; the CLI socket; `ui/demo/`; the ⌘ glyphs; the two flaky-by-locale tests isolated.
6. 🟡 **Pipeline the export**, index the mixer, `Arc` the bin in snapshots.
7. 🟡 **Speech, enhance, proxies and CBR as API verbs.**

Then HDR phase 4 (a true HDR preview) and phase 5 (zero-copy, hardware encoders).

## ✅ Keep

Layering · one compositor with the oracle in tests · `Session::open`'s three-way split · `clips_where` + `tidy_touched` · `follow_first_hdr` as an editor diff · mandatory digests with pinned upstreams · the updater's input hygiene · `Api::writable` · `OpenProjects` · `SingleFlight` · the lent compositor · Reverse's bounded two-pass design · tolerant `wire` readers · one contract, two transports · a hand-written CHANGELOG · CI that gates on fmt, clippy, perf and wasm.
