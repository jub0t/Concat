# Concat audit, 23 Sep 2026 — the quick version

Same audit as `AUDIT-2026-09-23.md`, one screen. Each bar has 10 slots:
🟢 filled = good, ⚪ empty = missing. Severity: 🔴 critical · 🟠 high · 🟡 medium · ⚪ low.

## 🎯 Verdict

**Structure: done. Guarantees: not yet.** The 16 Sep plan landed in full. What is left is rules the docs promise and the code does not hold yet: bounds, checks, one-place enforcement. Cheaper to fix than what came before.

## 📊 The whole app

| Aspect | Bar | Score |
|---|---|:-:|
| 🧹 Code quality | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 |
| 🏛️ Architecture | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 |
| 🧭 Design philosophy | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 |
| 🔧 Maintainability | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 |
| 😊 User likeability | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 |
| ⚡ Performance | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5 |
| 📈 Scalability | 🟢🟢🟢🟢⚪⚪⚪⚪⚪⚪ | 4 |
| 🔐 Security | 🟢🟢🟢🟢⚪⚪⚪⚪⚪⚪ | 4 |
| 🧪 Testing | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 |
| 📚 Docs | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 |
| 🤖 CI and release | 🟢🟢🟢🟢⚪⚪⚪⚪⚪⚪ | 4 |

## 🧱 Per crate (average of its six scales)

| Crate | Bar | Avg | One thing |
|---|---|:-:|---|
| concat-core | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 7.3 | 🟢 no deps, rational time · ⚪ linear clip lookup |
| concat-vision | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.2 | 🟢 ORT fully wrapped · ⚪ serial batch-1 |
| concat-effects | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.2 | 🟢 strict manifests · ⚪ no host-version or licence field |
| concat-text | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.0 | 🟢 zero engine deps · ⚪ copies the font per render |
| concat-api | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.6 | 🟢 every verb JSON, docs match · 🔴 any path, any size |
| concat-server | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7.2 | 🟢 constant-time token · 🔴 no limits at all |
| concat-media | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.7 | 🟢 unsafe is sound · 🔴 a panicking job wedges the scheduler |
| concat-project | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.0 | 🟢 commands only · 🟠 ripple copies every clip |
| concat-speech | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6.2 | 🟢 whisper on Metal · 🔴 downloads unverified |
| concat-export | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 5.8 | 🟢 one composite path · ⚪ 2 403-line lib.rs |
| concat-host | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 5.7 | 🟢 small modules · 🔴 corrupt doc opens empty |
| concat (Rust) | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 5.7 | 🟢 panes + echo-and-commit · 🔴 results land in the wrong project |
| concat (Slint) | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 6.8 | 🟢 one Modal pattern · ⚪ accessibility 2/10 |
| concat-render | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5.3 | 🟢 FramePlan · 🟠 7 CPU kernels for 147 shaders |

## 🎛️ Per feature

| Feature | Bar | Score | State |
|---|---|:-:|---|
| Launcher | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | ✅ |
| Import and bin | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | ✅ |
| Timeline editing | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | ✅ |
| Undo / redo | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | ✅ sharing lost on ripple |
| Relink | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | ✅ |
| Themes | 🟢🟢🟢🟢🟢🟢🟢🟢⚪⚪ | 8 | ✅ |
| Effects and filters | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | ⚠️ CPU path never pixel-compared |
| Titles | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | ✅ CPU-heavy |
| Keyframes | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | ✅ |
| Cutout | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | ✅ |
| Captions | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | ✅ |
| Text-to-speech | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | ✅ |
| Templates | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | ✅ |
| LUTs | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | ✅ |
| Export | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | ✅ serial loop |
| Hardware decode | 🟢🟢🟢🟢🟢🟢🟢⚪⚪⚪ | 7 | ✅ not zero-copy |
| Transitions | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | ⚠️ fades and wipes missing from preview |
| Crop, blend, masks | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | ⚠️ Darken/Lighten wrong at partial opacity |
| Enhance | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | ⚠️ one model, no native path |
| Playback and audio | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | ⚠️ 400 decodes per speed-curved clip |
| Remote API | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | ⚠️ unsafe defaults, window not a client |
| Languages | 🟢🟢🟢🟢🟢🟢⚪⚪⚪⚪ | 6 | ⚠️ 13 of 14 at ~71 %, no RTL |
| Custom packages | 🟢🟢🟢🟢🟢⚪⚪⚪⚪⚪ | 5 | ⚠️ trial never runs, multi-pass unwired |
| Proxies | 🟢🟢🟢🟢⚪⚪⚪⚪⚪⚪ | 4 | ❌ no UI, never swept |
| Accessibility | 🟢🟢⚪⚪⚪⚪⚪⚪⚪⚪ | 2 | ❌ two accessible-* properties |

## 🚨 Top findings

| # | Sev | What | Where |
|:-:|:-:|---|---|
| 0 | 🔴 | **`main` is red**: CI fails at fmt (8 files), 6 tests fail underneath | `ci.yml`, §1.2 of the full report |
| 1 | 🔴 | Corrupt project opens empty, next save overwrites it | `concat-host/src/session.rs:95` |
| 2 | 🔴 | Panicking scheduler job wedges the scheduler for good | `concat-media/src/prefetch.rs:355` |
| 3 | 🔴 | Worker results land in whichever project is open | `concat/src/studio.rs:5376` |
| 4 | 🔴 | API writes any path; input sizes and line lengths unbounded | `concat-api/src/lib.rs:499,707` · `concat-server/src/json.rs:117` |
| 5 | 🔴 | Window and Remote page are two engines on one folder | `concat/src/panes/settings.rs:403` |
| 6 | 🔴 | 20 of 23 model downloads have no digest; 18 URLs unpinned | `models/manifest.toml` |
| 7 | 🟠 | Hostile-package trial never runs; loop check trivial; no error scope | `concat-render/src/gpu.rs:850,1091` |
| 8 | 🟠 | Ripple, freeze, fill-slot deep-copy every clip into undo | `concat-project/src/model.rs:1966` |
| 9 | 🟠 | Stream start time ignored (MTS files offset) | `concat-media/src/decode.rs:743` |
| 10 | 🟠 | Darken / Lighten maths wrong at partial opacity | `concat-render/src/compositor.rs:258` |
| 11 | 🟠 | Art scan on every pointer event; full publish 30×/s | `concat/src/lib.rs:373` |
| 12 | 🟠 | CI is Linux-only until a tag; perf gate never runs; parity skips | `.github/workflows/ci.yml` |
| 13 | 🟡 | Clamps in four places, they disagree | `concat-project/src/commands/properties.rs:28` |
| 14 | 🟡 | E2E suite checks timing, not looks | `concat-host/tests/export.rs:915` |
| 15 | 🟡 | Caches with no bound: proxy, artwork, pinned frames, GPU passes | `proxy.rs`, `prefetch.rs:243`, `gpu.rs:1453` |
| 16 | 🟡 | 3–5 copies per frame; filter graph rebuilt per treated frame | `decode.rs:934`, `treat.rs:85` |
| 17 | 🟡 | Hub thread does heavy work in line; gRPC fan-out under a mutex | `hub.rs:52,71` |
| 18 | 🟡 | ONNX Runtime likely linked twice; pinned to an rc | `src/Cargo.toml:41` |
| 19 | 🟡 | Speech only reachable from the window | `crates/concat/Cargo.toml` |
| 20 | 🟡 | Titles copy the whole font per render | `concat-text/src/lib.rs:268` |
| 21 | ⚪ | Wrong H shortcut, ⌘ on every OS, "twelve" languages, dark colours in light theme | `settings.slint:671`, `tray.slint:127` |
| 22 | ⚪ | Dead `ui/demo/` still imported | `app.slint:21` |
| 23 | ⚪ | Stale docs: 429 tests (544), crate map, §7 diagram, mobile.yml, CONTRIBUTING | `ARCHITECTURE.md:276` |
| 24 | ⚪ | 8 files unformatted; dead `core::Project`; disk IO in a wasm crate | `timeline.rs:500`, `model.rs:1896` |

## 🧾 Today's checks

| Check | Result |
|---|---|
| fmt | ⚠️ 8 files would change |
| clippy | ✅ 0 warnings |
| tests | ❌ 537 passed, **6 failed**, 1 ignored (3 window, 2 render transitions, 1 Chatterbox) |
| perf --check | ✅ 14 of 14 within budget (GPU composite 3.2 ms, export 115 fps, hardware decode 326 fps) |

## 🗺️ Do next, in order

0. 🔴 **Make `main` green** — `cargo fmt --all`, then fix the 6 failing tests (5 are today's commits, 1 is locale coverage).
1. 🔴 **Correctness trio** — not-found vs parse error in `Session::open`; `catch_unwind` around scheduler jobs; a session epoch on worker results.
2. 🔴 **Server limits + root allowlist** — line cap, pre-auth timeout, connection cap, bounded outbox and sizes; share the window's exporter and sessions.
3. 🔴 **Verify every model** — fill the digests, pin to commits, reject empty digests in release, generate the tables from the manifest.
4. 🟠 **Refuse hostile packages for real** — trial at install, error scopes, binding-type check, pipelines keyed by source hash.
5. 🟠 **Document maths** — `clips_where` in place of `clips_mut`; one clamp pass; `start_time`; fix Darken and Lighten.
6. 🟡 **Stop the per-event work** — art refresh on change only; targeted publish; cache treat graphs; `av_frame_ref`; sweep proxy and artwork.
7. 🟡 **CI** — macOS and Windows check; lavapipe with parity required; `perf --check`; release depends on CI; fix the stale docs.

Then the ARCHITECTURE §10 list in its order: plan filled by the export → source textures pooled → zero-copy → one texture per track → native Enhance → multi-pass packages.

## ✅ Keep

Layering · `FramePlan` · commands-only document with Arc undo · hardware decode fallback · package load hygiene · JSON API contract and its docs · pane pattern · model staging and licences · tests that check behaviour.
