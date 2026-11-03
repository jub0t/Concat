## CI, release, distribution and GitHub. main eda5e0af, 2026-11-03

Labels as in the [engine report](engine-export.md).

GitHub data comes from the public REST API on 3 November, unauthenticated. Job logs need a signed-in session, so a CI failure is named here by its failing step, not by its error. Where I could, I reproduced the step locally.

### 1. CI on `main`

| Workflow, push to `main` since 24 Oct | Success | Failure | Cancelled |
|---|:-:|:-:|:-:|
| CI | **0** | 29 | 25 |
| Build for phones | 3 | 3 | 33 |
| Nix | 7 | 0 | 0 |

**C-H1. `main` has given no CI signal for ten days.** *Verified from the run list and failing steps; reproduced locally where noted.*

The failing steps, sampled across the period:

| Run (commit, date) | Failing steps |
|---|---|
| `94f4255`, 26 Oct | `cargo fmt --check`, clippy |
| `ad31fb3`, 27 Oct and `f0c0169`, 28 Oct | fmt, clippy, the Windows test run |
| `f19fda3` and `6c58ab7`, 2-3 Nov | fmt, clippy, **`cargo check` on macOS-15**, building the Windows tests |
| `eda5e0a`, 3 Nov (latest) | **"sherpa-onnx libraries"** in every engine job, and the same step on iOS and both Android jobs. Also fmt, and `scripts/models.py --check` |

- `e63756d` removed sherpa-onnx from the code, but `.github/workflows/build-app.yml` still fetches it (lines 125-162, 484-507), and so does the CI workflow. That step now fails before anything compiles, which hides whether `main` builds at all on macOS, Linux or the phones.
- Reproduced locally:
  - `cargo fmt --check`: 41 hunks in 12 files.
  - `scripts/models.py --check`: the model table still lists `kokoro-*` and `sherpa-onnx-pocket-tts-*`, which `tts.rs` no longer has, and does not list `system-voice`, which it does.
- Not reproduced: the clippy errors, the macOS `cargo check` failure and the Windows test build failure. The release build on this Windows machine compiled every library, so the non-test Windows code builds.
- 24 October recommended running fmt, the locale check and clippy before every push. Formatting drift has failed every run since.

**Fix:**
1. Drop the sherpa-onnx steps and matrix keys from both workflows, and remove the TTS rows from `scripts/models.py`.
2. Run `cargo fmt`.
3. Push once and let one CI run and one phone run finish.
4. The clippy and macOS errors then show up on their own.

### 2. Release

- v0.2.8 (20 October) is still the latest release. **195 commits on `main` have reached nobody.** On 24 October it was 117.
- v0.2.8 has 194 520 downloads:

  | Platform | Downloads |
  |---|---|
  | Windows | 81 531 (setup 71 565, MSI 9 966) |
  | Android | 65 552 |
  | iOS | 14 428 |
  | macOS | 12 879 |
  | Linux | 11 856 |

- 13 980 stars, 1 728 forks.

### 3. New findings

**C-H2. The identifier change makes the next phone build a second app, and the projects stay in the first one.** *Verified in the commit; Android's behaviour is as documented.*
- `b237687` renames the Android package and the iOS bundle from `app.concat.editor` to `org.standalones.concat`. The commit itself says "Android treats the new package as a different app: it installs beside the old one, and the old one's data stays in its own sandbox", and that iOS behaves the same way.
- The desktop is handled well: `dirs::adopt` moves the old folder and leaves a junction or symlink behind (`concat-host/src/dirs.rs:167-185`). The installer's `AppId` is unchanged, and the Inno script deletes no app data on uninstall, so the junction is safe.
- On phones nothing is carried over. That covers 65 552 Android and 14 428 iOS downloads of 0.2.8: their projects, recents, downloaded models and saved voices.
- 0.2.8's in-app updater will offer the new APK as an update. Installing it gives the user a second, empty Concat.
- 24 October's step 4, "register as an Android developer for `app.concat.editor`", now applies to the new name.
- **Fix:** pick one before the next phone release:
  - Keep `app.concat.editor` on Android and iOS, and change only the desktop folders.
  - Or ship one last build under the old identifier that writes its projects somewhere the new app can import them from, and say so in the release notes and in the update prompt.

**C-M1. Triage has fallen behind.** *Verified.*
- 70 issues were opened since 24 October. **50 of them are open with no comment at all**, including #533 (labelled `priority`) and #556, #561 and #600, three GPU crashes on 0.2.8. Eight have no labels.
- 27 PRs are open. Fixes from contributors that are waiting:
  - #646: MCP export reports done at frame 135 of 150, for #595.
  - #620: halation on DX12.
  - #584: audio switches out of sync.
  - #582: effect chain passes.
- Four large "close the audit findings" PRs from one account (#610, #612, #613, #624) were closed unmerged. No prompt injection was found in the PR titles or issue bodies sampled.
- The Dependabot cargo-group bumps fail (6 runs). `zip` fails three times on its own.
- The `Mirror models` workflow failed on 26 October.

### 4. The 24 October distribution findings

| 24 Oct | Now |
|---|---|
| O1: #533, Smart App Control blocks unsigned DLLs | **Unchanged.** The LGPL switch (`b8b6528`) swaps one unsigned BtbN build for another. Microsoft Store (#645) and own-built FFmpeg are both still open |
| O2: unsigned update manifest | **Unchanged.** `c2847c1` routes downloads through the system proxy, but the updater keeps `Trust::Shipped` (`concat-host/src/updates.rs`, `net.rs:283-284`), so 41ca63a's fix stands |
| O4: commits that reach nobody | **Worse:** 195 |
| H6: macOS floor | Partly. #625 (Monterey, libavdevice) is closed. #600, a Metal shader error on an Intel iMac, is new |

### 5. Scores

| Aspect | 24 Oct | Now | Why |
|---|---|---|---|
| CI and release | 4 | **2 ↓** | 0 green CI runs in 54, 3 green phone runs in 39, no release in 14 days |
| Distribution and trust | 2 | 2 | No signing, no store. The identifier change splits the phone install base, while the LGPL builds remove one obstacle to a store |

### 6. Not checked

- The CI logs themselves.
- Whether 0.2.8's Android updater really downloads and launches an APK with a different package. That is reasoned from the manifest and package names.
- The website and the release notes.
