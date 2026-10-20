# Concat audit, 20 October 2026: the quick version

The one-screen scorecard for [`AUDIT-2026-10-20.md`](AUDIT-2026-10-20.md).
Code at `c41865e`, 175 commits past the 15 October audit, with v0.2.8
published today (tag `79116bd`). The reviewers' own reports are in
[`2026-10-20/`](2026-10-20/).

## In one paragraph

0.2.8 is the biggest release yet and the first whose notes are honest
about what was not tested. About 205 000 people have downloaded a Concat
build, 78% of them on Windows or Android. The engine is in better shape
than the shell: the server work two audits asked for landed, phones now
decode on MediaCodec, and 984 tests pass with clippy clean. But the things
that lose a user in the first ten minutes are still there. A single early
crash puts the app on the software renderer for good. Android's Back
button quits mid-export. A tap on a clip moves it. Windows is unsigned and
in no store. CI on `main` has been red or cancelled for most of a week, so
it tells nobody anything. And none of today's fixes are in any release.

## Scores

| Aspect | 15 Oct | Today |
|---|:-:|:-:|
| Code quality | 6 | 6 |
| Design architecture | 7 | 7 |
| Design philosophy | 5 | 6 ↑ |
| Maintainability | 5 | 4 ↓ |
| User likeability | 6 | 6 |
| Performance | 5 | 6 ↑ |
| Scalability | 5 | 5 |
| Security | 5 | 5 |
| Testing | 5 | 6 ↑ |
| Documentation | 6 | 5 ↓ |
| CI and release | 4 | 5 ↑ |
| Distribution and trust (new) | — | 3 |

## The numbers

| | Today | Δ since 15 Oct |
|---|---:|---:|
| Rust lines, every `.rs` under `src/crates` | 121 054 | method changed, not comparable |
| Slint lines | 34 913 | +4 832 |
| `#[test]` functions | 988 | +210 |
| `cargo test --workspace` | 984 passed, 0 failed, 3 ignored | was 773/1/1 |
| clippy `-D warnings`, fmt | pass | |
| `locales.py --check` | pass (881 keys; 707 values still English) | was failing |
| `unsafe` (grep) | 138 | +28 |
| `studio.rs` | 12 739 lines | +1 578 |
| `start()` | 2 240 lines | +370 |
| Effect packages | 127 (102 picture, 25 audio) | |
| CI on `main` | red or cancelled most of the week; last two pushes running | |
| Open issues / PRs | 141 / 16 | +35 / +10 |
| Downloads, all releases | ~205 000 | v0.2.7 alone 126 218 |

## Fixed today (on `main`, in no release yet)

#418 Windows start crash from a stray `dxcompiler.dll` (b18c2fe, a9b2df6) ·
garbled-window escape, "Concat (safe graphics)" (64e9dcd) · phones reach
every transition and filter shelf (#415, a1fd411, d4ced41) · effect cards
ship inside the app (#416, f2d0637) · transition knobs settable (ce64dae,
d4ced41) · File › Open no longer drops the open project unsaved and leaks
its lock (2abfcf7) · Android engine library aligned for 16 KB pages, with
a CI check (c41865e) · shortcut reset (be60358) · private vulnerability
reporting switched on (it was off while SECURITY.md pointed at it).

## Open, High

| # | What | Where |
|---|---|---|
| H1 | Any death in the first 15 s puts every later launch on WARP/lavapipe, export included, for good; `--smoke` leaves the mark too | `concat/src/lib.rs:2454-2475, 2381` |
| H2 | Android Back quits the app with a sheet or an export open; nothing is saved when the activity ends | `app.slint:504-539`, `lib.rs:2433` |
| H3 | Phone: a 4 px wobble on a clip moves it with an undo step; a pinch selects and can move; targets 24-36 px | `lanes.slint:591`, `studio.rs:4804-4978` |
| H4 | CI on `main` is no signal: phone runs cancelled 50 of 60 times, lint steps abort before tests | `mobile.yml:92`, `ci.yml:136-141` |
| H5 | Unsigned on Windows, ad-hoc on macOS, in no store; the updater trusts a digest from the release it checks | `build-app.yml:1036`, `updates.rs` |
| H6 | macOS bundles say 12.0; CI shows linked libraries needing 14 or 15 and only warns | `build-app.yml:96, 294-302` |
| H7 | Export recovery opens a second high-performance DX12 device, the road #202 says crashes hybrid NVIDIA laptops | `concat-export/src/lib.rs:1530` |
| H8 | Captions start early (#438): no voice detection, no token timestamps | `concat-speech/src/transcribe.rs:481-545` |

## Do first

1. Ship today's fixes: #418 and the 16 KB alignment are start-up failures in the wild.
2. Point the website at 0.2.8 and fix its checksum link (#458).
3. Safe start: trip only on a crash, retry the GPU, clear the mark on `--smoke`.
4. Android Back closes the top sheet; save on pause and destroy.
5. Phone timeline: long-press to move, 8 dp slop, 44 px targets.
6. CI: lint in its own job, no cancelling on `main`, phone runs required.
7. Windows signing or the Microsoft Store; Android developer registration.
8. An honest macOS floor, and a failing minimum-OS check.
9. Land D3monX's caption timing PRs once they're up.
10. Merge the four approved PRs (#445, #446, #441, #424): blocked on a permission only the owner can grant.
