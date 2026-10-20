# Product, community and distribution audit, 20 October 2026

Repo `jub0t/Concat` at `ce64dae`. v0.2.8 was published at 2026-10-20T08:18:58Z from tag `v0.2.8` = `79116bd` (2026-10-18). Its assets were uploaded at 2026-10-19T03:38Z. Sampled 08:40–10:50Z. Read-only: `gh` reads, `curl`, `apksigner` on downloaded APKs (stored in the scratchpad, not the repo). Issue and PR text was treated as data.

Legend: **V** = verified here; **U** = unverified; **R** = repeated from `reports/` and not re-checked.

## Bottom line

Demand is huge, and distribution throws much of it away. v0.2.7 drew 126k binary downloads in a week. The v0.2.8 launch is being undercut by Concat's own website. The "Start here" site still serves **v0.2.7 links** next to a `latest/` checksum file that now lists **0.2.8** names. In one 15-minute window, v0.2.7 gained as many downloads as v0.2.8 did (+68 vs +75).

Concat sits in no store or package manager it runs itself. Windows is unsigned, macOS is not notarised, iOS is ad-hoc, and the Android signing key changed between 0.2.5 and 0.2.6. Two-thirds of outside-filed open issues have no maintainer reply. Three issues marked `testing-requested` today have their fixes **only after the v0.2.8 tag**, so no reporter can test them. SECURITY.md points reporters at GitHub's private advisory form, but **private vulnerability reporting is disabled**.

The smaller rival Drift is already on Flathub, the Microsoft Store, a Homebrew tap and Google Play closed testing.

## 1. Releases and reach

### Downloads (V, `gh api repos/jub0t/Concat/releases --paginate`; manifest.json and SHA256SUMS excluded)

| Release | Published | Total | Windows | Android | iOS | macOS | Linux |
|---|---|--:|--:|--:|--:|--:|--:|
| v0.2.8 | 10-20 08:18Z | 584 → 659 at 10:50Z | 295 | 163 | 10 | 52 | 64 |
| v0.2.7 | 10-13 | 126,218 | 54,353 (43.1%) | 44,190 (35.0%) | 10,279 (8.1%) | 9,242 (7.3%) | 8,154 (6.5%) |
| v0.2.6 | 10-06 | 11,328 | 5,155 | 3,323 | 243 | 793 | 1,814 |
| v0.2.5 | 09-28 | 42,954 | 17,665 | 15,206 | 3,487 | 3,512 | 3,084 |
| v0.2.4 | 09-24 | 4,364 | 2,049 | 833 | 195 | 284 | 1,003 |
| v0.2.3 | 09-19 | 11,846 | 5,919 | 2,718 | 716 | 1,126 | 1,367 |
| v0.2.2 | 09-15 | 3,316 | 2,017 | 535 | 59 | 332 | 373 |
| v0.2.1 | 09-04 | 4,277 | 2,567 | 913 | 81 | 423 | 293 |
| **All** | | **≈204.9k** | | | | | |

- Per asset for v0.2.7: `windows-x86_64-setup.exe` 48,897, `android-arm64.apk` 44,190, `ios-arm64.ipa` 10,279, `macos-arm64.dmg` 7,498, `.msi` 5,456, `x86_64.AppImage` 2,546, `linux-x86_64.deb` 2,247. The Arch `.pkg.tar.zst` got 703 across both arches.
- v0.2.8 per asset: setup.exe 244, apk 163, msi 51, macos-arm64 45, AppImage 21+15, deb 16+3, ipa 10.
- **The in-app updater is barely used (V).** v0.2.7's `manifest.json`, which only the updater fetches (`src/crates/concat-host/src/updates.rs:417`), was downloaded 1,339 times against 126k binaries, about 1%. Updates are checked only when the user opens Settings › Version (`src/crates/concat/src/panes/settings.rs:509`). That fits the privacy policy, but nobody is told a new version exists, so each release has to win its audience back from scratch.
- Windows ARM was dropped after v0.2.6 (534 + 153 downloads that release; #326). The x64 setup still installs on ARM under emulation (`assets/windows/concat.iss:15`, `ArchitecturesAllowed=x64compatible`).

### Repo (V, `gh api repos/jub0t/Concat`)

9,638 stars, 1,161 forks, 79 watchers, 157 open issues+PRs (141 issues, 16 PRs), AGPL-3.0, created 2026-08-25. Homepage field: `concat.standalones.org`.

I sampled the last 100 stargazers (03:59–10:27Z today, about 100 stars in 6.5 h). 6 accounts were under 30 days old and 16 had no repos and no followers. That looks organic, not bought (V, sample only).

### Traffic, 14 days (V, `/traffic/*`)

- **Views:** 166,377, 91,011 unique. The peak was 26,800 on 10-16; daily views were above 19k on 10-15 to 10-18 and fell to 14,423 on 10-19.
- **Clones:** 11,098 (3,441 unique).
- **Referrers:** Google 41,694 (33,424 unique); github.com 21,441; Facebook (l./lm./www.) 11,202 combined; Brave 2,343; `concatenate.pages.dev` 2,228; Bing 1,909; t.co 1,219; youtube.com 1,180.
- **Paths:** `/jub0t/concat` and `/jub0t/Concat` are about 105k combined. `/releases` 15,520 and `/releases/tag/v0.2.7` 6,417. `docs/README.es.md` is the 10th most-viewed path (1,962 views), so Spanish speakers are a real segment.

## 2. Distribution and trust

### Where Concat is carried

| Channel | Status | Evidence |
|---|---|---|
| GitHub Releases | Yes, 16 assets / 5 OSes | V |
| Official site | `concat.standalones.org` (in-app `SITE`, `src/crates/concat/src/panes/start.rs:94`) and `concatenate.pages.dev` (README). Same HTML. **Both link only v0.2.7 assets** (47× "0.2.7", 0× "0.2.8"), plus `releases/latest/download/SHA256SUMS`, which is now 0.2.8's file. Anyone checking the hash of a site download gets a **mismatch**. Owner issue #458 ("website: update ... out-dated data") is open and unlabelled. | V (`curl`, 10:4xZ) |
| winget | **No** (`microsoft/winget-pkgs/manifests/j/jub0t` and `.../c/Concat` 404; code search empty). `concat.wxs:9` says the .msi is for "winget" deploys, but there is no manifest. | V |
| Microsoft Store | **No.** Removed in `be1b882` (2026-10-12, "no Microsoft Store"). Repo **variables** `MSSTORE_IDENTITY_NAME=jub0t.Concat`, `MSSTORE_PUBLISHER`, `MSSTORE_PUBLISHER_DISPLAY_NAME` are still set (`gh variable list`), so the name reservation probably still exists. | V |
| Google Play | No (`play.google.com/...id=app.concat.editor` 404) | V |
| F-Droid / IzzyOnDroid | No (both APIs 404) | V |
| Flathub | No (`flathub.org/api/v2/appstream/app.concat.editor` 404) | V |
| Flatpark | Yes, third party. Its page says "community package ... not affiliated", "repackages the official upstream Linux x86_64 .deb ... unmodified", still **0.2.7**, "127 new installs in 30 days". The README lists it with a ✅ Linux line. | V (WebFetch) |
| Homebrew | No (`formulae.brew.sh/api/cask/concat.json` 404) | V |
| AUR | Community: `concat`, `concat-bin` (3 votes), `concat-git`, maintained by **Felitendo**, still at **0.2.7-1** (2026-10-16) | V (AUR RPC) |
| App Store / TestFlight | No. The `.ipa` is ad-hoc signed and needs re-signing by the user (`.github/workflows/mobile.yml:30,414-430`) | V |

### Code signing per platform (V: `gh secret list` shows only `ANDROID_KEYSTORE` and `ANDROID_KEYSTORE_PASSWORD`)

| Platform | State | Evidence |
|---|---|---|
| Windows | **Unsigned.** `build-app.yml:1032-1037` prints "No signing certificate: the build is unsigned" because `WINDOWS_CERTIFICATE_PFX` is unset. SmartScreen warns, and Smart App Control **refuses** to run it (CODE_SIGNING.md:3-6). SignPath is test-signing only (maintainer). CODE_SIGNING.md never mentions SignPath. | V |
| macOS | **Ad-hoc, not notarised**: `build-app.yml:698-699` `codesign --force --deep -s -` (no `APPLE_*` secrets). The README tells users to run `xattr -dr com.apple.quarantine`. | V |
| iOS | Ad-hoc; the user signs it with their own Apple ID (AltStore/Sideloadly). 10,279 people downloaded the v0.2.7 IPA. The site offers it to casual users directly, and #338 ("I can't download it on ios") is unanswered. | V |
| Android | Signed with the repo key: `CN=Concat, O=Concat`, cert SHA-256 `923308579c0b6f4d6d9be39c504f5f05f248feeae41d5caece23f058a29d6e2e`, v2+v3, for v0.2.6/0.2.7/0.2.8. **v0.2.5 was signed with a different key** (`4e6bdfb2…b042`), so the 15,206 people who installed v0.2.5 cannot update in place: Android refuses a different signer, and they must uninstall and lose app data. No release note found warns about this (U whether any was ever posted). | V (`apksigner verify --print-certs` on all four APKs) |
| Linux | Unsigned packages. SHA256SUMS is unsigned (no GPG/minisign/cosign). | V |
| All | **No build provenance or attestations**: no `attest`/`sbom`/`cosign`/`id-token` anywhere in `.github/workflows/`. The updater checks bytes against a digest in `manifest.json` from the **same release** (`updates.rs:6-13`), so one compromised GitHub token or account can ship code to every updater. On the positive side, all third-party actions are SHA-pinned. | V |

### Android developer verification readiness

- Google's schedule: enforcement for **Brazil, Indonesia, Singapore and Thailand from 2026-09-30**, global in **2027**. On certified devices, normal installs of apps from unregistered developers are blocked; adb and a slowed "advanced flow" remain ([developer.android.com](https://developer.android.com/developer-verification/guides?hl=en); [The Hacker News](https://thehackernews.com/2026/06/google-sets-sept-30-deadline-for.html?hl=en); [Help Net Security](https://www.helpnetsecurity.com/?p=375237)). Whether the 30 September enforcement actually started on time is **U**: I found no post-deadline reporting.
- Concat has **no sign of registration**. The repo, docs and issues contain no mention of developer verification (V, grep).
- **It is feasible:** the package name is `app.concat.editor` and the key has been stable since 0.2.6 (V), which are the two things verification ties to an identity.
- **Exposure:** Indonesia is plausibly a real market. There is an Indonesian locale, Indonesian contributors (PR #383 Indonesian TTS, @dapaaa07) and an Indonesian README. Country-level download data does not exist, so the size of the impact is **U**.
- Other Android gaps (V, `aapt2 dump badging`):
  - The APK is **arm64-v8a only**, so 32-bit phones (#392, unanswered) and x86_64 Chromebooks are out. Drift ships arm64, armeabi-v7a and x86_64.
  - targetSdk 35. That is fine for sideloading, but Play would want 36 after Aug 2026 (U, based on Google's annual rule).
  - 0.2.8 adds `RECORD_AUDIO` (voiceover). Otherwise it only asks for `INTERNET`.
  - The release notes admit "no automated check has started this APK": CI's emulators are x86_64 (`mobile.yml:34-43,106`).

### Installer behaviour (Windows)

- `assets/windows/concat.iss`: per-user by default (`PrivilegesRequired=lowest`, can be raised from a dialog), 25 installer languages vendored, desktop icon opt-in, launches after install.
- `assets/windows/concat.wxs`: per-machine (`Scope="perMachine"`), Start menu only, no UI, and no desktop icon.
  - The two installers have different AppId/UpgradeCode values. Installing both gives **two copies and two "Concat" Start entries** (U, by construction).
  - The updater does pick the right installer per install (`updates.rs:280-290,491-504`, V).
- **"Concat (safe graphics)"** (`64e9dcd`, 2026-10-20 09:27Z, added to both installers):
  - It is **not in any release**. It landed an hour after v0.2.8 was published, and the tag is `79116bd`. Users hitting #389/#449 (garbled window) have nothing in hand today.
  - It is **sticky**. The commit says the entry "turns the switch off before the window opens and keeps it off." One click leaves a GPU video editor on the software rasteriser forever, including from the normal shortcut, until the user finds Settings › General › Draw on the GPU. Expect "Concat is slow" reports that are really this setting.
  - The shortcut is always created (not optional), so every install gets a second, confusing Start entry.
- Separately, `b18c2fe` (post-tag, #418) pins D3D12 to **FXC**. That partly reverses `be1b882`, which moved to DXC because FXC "takes minutes over the Gaussian loops of the blur and the glow" (#297). That trade-off needs proving before v0.2.9 (U; the code is outside this area).

### Impersonation and lookalikes (flag only, nothing done)

- **`github.com/Concat-video-editor`** is an Org created 2026-10-16 (the day of the traffic peak).
  - Its `.github` profile README is SEO filler: "Concat - Online Video Editor for Fast Browser Editing", "a practical Concat CapCut alternative".
  - It **hotlinks `concat.standalones.org/_astro/editor…webp`** and gives no link to `jub0t/Concat`.
  - This is name squatting or SEO spam riding Concat's name (V, `gh api orgs/Concat-video-editor`). `TRADEMARK.md` exists, and the owner can report it to GitHub.
- There are **two official domains** (`concat.standalones.org` canonical, `concatenate.pages.dev` in the README). Two names make phishing lookalikes easier to pass off; pick one.
- README "Paid Sponsors" promotes Proxyon (proxies, `JUB0T` code). That is a trust-optics risk for an app sold on privacy. This is opinion.

## 3. GitHub state

### Issues (V, GraphQL over all 141 open)

- **Labels:** feature request 70, bug 51, testing-requested 32, ui-ux 27, important 24, windows 22, priority 19, timeline 17, video 16, crash 15, effects 14, android 13, audio 13, captions 12, improvement 12, gpu 10, AI 9, export 8, ios 8, editor-api 7, distribution 6, linux 6, text 6, needs-info 5, performance 5, macos 4, plus smaller ones. 4 are unlabelled: #455, #457, #458 (owner) and #459 (outside, today).
- **Per platform (open):** Windows 22 (18 bug, **10 crash**), Android 13 (5 bug), iOS 8 (3 bug, 1 crash), macOS 4 (2 crash), Linux 6 (1 crash).
- **Authors:** 33 owner-authored, 108 by others.
- **No maintainer reply: 70 of 108 outside issues (65%).** The owner wrote 77 of all comments on open issues.
  - The oldest unanswered issue is #155 (2026-09-19, 31 days). The oldest with zero comments at all is #200 (09-25).
  - **Oldest unanswered crash: #294 "App crash at launch in IOS"** (10-09, 0 comments).
  - Also unanswered: #313 (AVX2 launch crash, 10-12), #324 (Linux Vulkan export device loss, priority), #343 (Ubuntu preview won't play), #371 (Intel Xe renders no UI), #423 (macOS legacy AMD launch failure), #437 (⌘Q loses edits), #449 (green screen, Windows), #338 (iOS can't download).
- **`testing-requested` (32): the process is broken.**
  - The labels went on today between 08:41 and 09:56Z, after v0.2.8 was published (e.g. #313 at 09:55:53Z, #372 at 08:42:29Z, via the timeline API).
  - **14 have zero owner comments** (#213, #239, #244, #253, #286, #313, #347, #350, #355, #356, #372, #413, #415, #416). A label change does not notify the reporter (U, but GitHub does not send notifications for label events), so these reporters have not been asked to test.
  - **#415, #416 and #418 have their fixes only after the tag** (`a1fd411`, `f2d0637`, `b18c2fe`/`a9b2df6`). No downloadable build contains them, and #418 is a Windows startup crash.
  - #86, #395, #413 and #434 have no commit that references them.
  - #223 and #297 were confirmed by their reporters (R) and are still open.
- **Discussions:** 29 in total. 8 of the 12 newest have 0 comments (D#398, D#387, D#384, D#376, D#366, D#298 …).

### PRs (V, `gh pr list`, reviews)

- **16 open.** Reviews posted today 10:04–10:09Z are substantive and correct. For example, #383 was blocked because the Piper archive's `MODEL_CARD` describes a Malayalam voice, so its `licence = "MIT"` has nothing behind it. #360 was told to drop the dead `vn.json`.
- **Approved but unmerged:** #446, #445, #441 (full CI green: 10 checks), and **#424 and #420 approved with only GitGuardian run, no CI**. CODE_SIGNING.md promises review before merge; review happened, testing did not.
- **Stale contributor work:**
  - **#157** (Mehdidjah, "geometric masks and motion tracking", +4,494, opened 09-19) has **zero reviews in 31 days**. Meanwhile the owner shipped his own shape masks in 0.2.8 (#239), which silently supersedes it.
  - #358 (template gallery, draft since 10-15). #425/#426 (MCP drafts, 10-18).
  - Dependabot #411 (21 cargo bumps) and #368 (6 action bumps) are unreviewed.
- **Closed PRs:** 117, of which 71 merged and 46 not. 43 of the unmerged are outside contributors'. #300, #187 and #275 were closed with **no comment or review at all**.
  - #300's work was ported as `b12e527` with authorship and sign-off kept (good), but the PR itself got no word.
  - The v0.2.8 release notes have **114 bullets and zero contributor credits** (no `@` in the body).
- **Git authorship:** 936 + 55 commits by Jareer. Co-author trailers: 349 Claude Fable, 255 Claude Opus 5.5, 14 Cursor.
- **Translations:** 133 of about 157 commits to `src/crates/concat/locales/` are by Jareer. They are effectively machine translations with little native review (U for quality).

### Trolling, spam, prompt injection (flag only)

- I scanned every open issue, comment and PR body for injection phrasing, hidden HTML, shorteners, file-sharing links and crypto terms. **Nothing hostile found.**
  - Hits were benign: users saying they used ChatGPT or Claude (#131, #318, #389), pasted HTML (#388), and dependabot bodies.
  - Attachments are `.log`/`.txt` files on `github.com/user-attachments` (#388, #389, #395, #449). Treat them as untrusted when opened.
- Low-value or vague issues: #307 "Feedback", #362 "features request", #393 "transition add", #408 "Request New Feature".
- **Owner's own #363** proposes to "reverse engineer and allow using assets from already installed CapCut". That is a copyright and ToS liability; ByteDance owns those packages. Flagged for the owner.
- External squatting: see `Concat-video-editor` above.

## 4. Product claims vs reality

README.md at HEAD; release notes from the v0.2.8 body.

| Claim | Verdict | Evidence |
|---|---|---|
| About: "for macOS, Windows, Linux and Android"; Highlights: "macOS, Windows, Linux, Android" | **Inconsistent.** Platforms lists "✅ iOS / iPadOS", and the site serves the IPA (10k downloads). | README.md:46,77,92 |
| "✅ Windows" | **Overstated.** Unsigned (Smart App Control blocks it outright), 10 open crash issues, and the #418 startup crash fix is in no release. | §2, §3 |
| "✅ Android · phones and tablets" | **Overstated.** arm64 only; the release notes say no automated check has launched the APK; the GL ES fallback is "only run on an emulator". | notes "Before you install"; `aapt2` |
| "✅ iOS / iPadOS · sideloaded" | **Overstated; should be 🧪.** Ad-hoc IPA, #294 launch crash unanswered, #338 unanswered. | §2, §3 |
| "✅ Linux ... Also a Flatpak on Flatpark" | True but third party, unaffiliated, and still 0.2.7 | Flatpark page |
| "4K scrubs smoothly" | **Unverified, and contradicted for many users.** No 4K or scrub benchmark or test in the repo (grep of `crates/*/tests`, no benches). Open #261 "Massive Performance Issues", #149, #412, #343. Android decoded on the CPU until 0.2.8 ("Video decodes on MediaCodec, which it never did"). | V |
| "100 effects, filters and transitions" | **True by count:** 23 effect + 57 filter + 22 transition = 102 visual packages (+25 audio) in `src/crates/concat-effects/packages/*/effect.toml`. **But in v0.2.8, 17 of 22 transitions declare knobs no user can set** (V: 15 have 1 param, 2 have 2). That was fixed only post-tag in `ce64dae`. The "just a fade" test `4451548` is also post-tag (#448). | V |
| "25 languages" | **True by file count:** 25 locales × 882 keys, none missing. **Overstated in practice:** de 87, fr 93, it 67, es 63 strings identical to English (some legitimately); hard-coded English (#352, PRs #424/#360); effect names English-only by design (#364); no UI mirroring for ar/he/fa/ur found (bidi exists only in caption text, `concat-text/src/lib.rs:983-1010`). | V (script over `locales/*.json`) |
| "100% local. Nothing uploads." / "no telemetry" | **Consistent with code checked.** Update check is manual; models are fetched on request; start-page cards are links. | `updates.rs`, `settings.rs:509`, `start.rs:94-110` |
| "Export H.264, HEVC, AV1. Up to 4K 60, 10-bit" | **Partly verified.** Codec names and HDR 10-bit are in `concat-export/src/lib.rs:418-486`. Per-platform encoder availability is U. #286 (Android missing codecs) is testing-requested with no reply. | V/U |
| "Auto-captions. Local Whisper ... styled" | Works, with accuracy caveats: #438 captions land seconds early; #325 asks for a larger model. | V (issues) |
| "Same app, same project files" | U | |
| "Download step 1: The website hands you the right build" | **False today.** The site hands out 0.2.7 and a mismatched checksum link. | §2 |
| Version badge "0.2.8" | True for GitHub; false for the site | |
| Notes: "Q and E cut the selected clip at the playhead" | **False in 0.2.8.** It was superseded by `dce4ad0`: Q = delete left, W = delete right, **E unbound** (`keymap.rs:116-120`, tests at `:613-615`). docs/KEYBOARD_SHORTCUTS.md:26-27 is correct. The notes contradict themselves. | V |
| Notes: "Intel Macs open on macOS 12 again (#372…)" | **Unverified.** Reporter never contacted, labelled testing-requested today. | §3 |
| Notes: "Delete left/right … (#400)", "Stabilize" (#253), masks (#239) | Code is in the tag (V); no reporter confirmation (U) | |
| Notes download table | Omits the shipped Arch `.pkg.tar.zst` | V |
| Notes as a whole | 114 bullets, many internal ("the lock covers one far byte", "uniforms come in whole 16 bytes", "Rational::parse refuses a value past i64"). Unreadable for the CapCut-refugee audience. | V |
| CODE_SIGNING.md: "Every change from anyone else ... is reviewed by a reviewer before it is merged" | True for review. But #424/#420 were approved without CI, and "Every member uses multi-factor authentication" is U. | V/U |
| SECURITY.md: "Report it privately through GitHub's advisory form" | **Broken.** `gh api repos/jub0t/Concat/private-vulnerability-reporting` → `{"enabled":false}`. | V |

## 5. Competitive position

| | Concat | CapCut | Drift (CutWire-Studios) | OpenCut | Kdenlive / Shotcut |
|---|---|---|---|---|---|
| Licence / stack | AGPL-3.0, Rust + Slint + wgpu (V) | Proprietary (ByteDance) | GPL-3.0, C++/Qt 6 + FFmpeg (V) | MIT, TypeScript web, Rust rewrite in progress (R) | GPL-3.0, C++ (V) |
| Stars | 9,638 (V) | n/a | 2,039 (V, created 2026-07-07) | 95,433 (V) | 5,904 / 15,456 (V) |
| GitHub downloads | ≈204.9k all-time; 126k for v0.2.7 (V) | n/a | 94,777 all-time; v0.8.0 9,702 (V) | Releases are FFmpeg builds only (V) | Kdenlive 11,500,714 official downloads in 2025 (V, [kdenlive.org](https://kdenlive.org/news/2026/state-2026/)) |
| Platforms | Win x64, macOS, Linux, Android arm64, iOS ad-hoc (V) | Win, macOS, Android, iOS, web | Linux, Windows, macOS (Apple silicon), Android arm64/armv7/x86_64 (V, assets) | Web (R) | Desktop only; Kdenlive mentions no mobile app (V) |
| Stores / package managers | **None first-party**; AUR and Flatpark community, both 0.2.7 (V) | All major stores | **Flathub (16,880 installs, V), Microsoft Store (listing 301 → V exists, U content), Homebrew tap (V), Play closed testing (Drift README, U)**, portable zip (V) | n/a | Kdenlive: Flathub etc.; Microsoft Store "soon" (V) |
| Signing | Unsigned Windows, ad-hoc macOS (V) | Signed | Ad-hoc macOS by its own README (V); Windows via Store (U) | n/a | Signed Windows (U) |
| Price / catches | Free, no account, no watermark (V) | Pro $179.99/yr, up from ~$77–90 ([Newsweek](https://newsweek.com/app-used-millions-nearly-doubles-subscription-price-overnight-11535999), V across sources); content-licence terms (R) | Free, no account, no watermark (Drift README) | Free | Free; Kdenlive €9,344.80 donations, 2 developers (V) |
| AI | Local Whisper captions, TTS + voice cloning, cutout, enhance (V code) | Cloud plus on-device | Auto captions, upscale, face retouch (Drift README, U) | U | Shotcut Whisper; TTS needs Docker (R) |
| Automation | MCP + JSON-RPC (V) | No | U | U | MLT XML |

Read-across:
- Drift is the direct threat on distribution, not features. It is smaller and younger, yet installable from three stores and is first in line on Android.
- The report's claim that there is "no open-source rival with a mobile app" (`reports/What Concat users want most.md`) is **false**: Drift ships Android APKs (V).

## 6. Scores

| Aspect | Score | Justification |
|---|:-:|---|
| User likeability | **5** | Real CapCut-parity gains in 0.2.8 (delete L/R, masks, stabilize, SRT, voiceover) and huge demand. But 65% of outside issues are unanswered, 10 Windows crash issues are open, iOS crash reports go unanswered, and the Android APK has never been launched by CI. |
| Documentation | **5** | Effect and locale counts are honest and the release-note caveats are candid. But SECURITY.md points to a disabled channel, the README platform claims contradict each other, "4K scrubs smoothly" has nothing behind it, the website is a release behind with a wrong checksum link, and the notes are 114 jargon bullets including a false keybind. |
| CI and release (distribution angle) | **4** | 16 artifacts across 5 OSes with SHA256SUMS and SHA-pinned actions. But the website was not updated with the release, testing-requested fixes are missing from the build, there is no notarisation, no signed checksums and no provenance, and two PRs were approved with no CI. |
| Security (supply chain / licences / trust) | **4** | Dependabot, push protection, model licences declared, and a sharp licence review on #383. Against that: unsigned Windows, ad-hoc macOS, an updater trusting a same-release digest, no attestations, a disabled vulnerability-reporting channel, and an Android key rotation that stranded 0.2.5 users. |
| **Distribution and trust (new)** | **3** | GitHub plus a stale site only. Not on winget, Store, Play, F-Droid, Izzy, Flathub or Homebrew; community AUR and Flatpark lag a release. Unsigned or ad-hoc on three OSes, no Android developer verification while enforcement has started in four countries, and an SEO squatter using the name. A younger rival is in three stores. |

## Highest-value fixes (ordered)

1. **Today:** point the site's download links at v0.2.8, or better, make them derive from `releases/latest`, and fix the SHA256SUMS mismatch.
2. **Close the `testing-requested` loop.** Post one comment per issue saying "fixed in 0.2.8, please test on <asset>". Un-label #415, #416 and #418 until a build contains their fixes.
3. **Turn on private vulnerability reporting** (Settings › Security). It is a single toggle.
4. **Register as an Android developer** for `app.concat.editor` with the current key. Submit to IzzyOnDroid, which takes developer-built APKs.
5. **Get onto the stores.** Microsoft Store registration is free for individuals and Microsoft signs Store apps (R, [Windows Developer Blog](https://blogs.windows.com/windowsdeveloper/2025/09/10/free-developer-registration-for-individual-developers-on-microsoft-store/)); the `jub0t.Concat` identity is still configured. Then a winget manifest for the .msi and a Flathub submission (Drift proves both work for this category).
6. **Fix the README platform marks.** Mark iOS 🧪, add Android "arm64 only, untested on real devices in CI", and drop "4K scrubs smoothly" until a benchmark exists.
7. **Make "safe graphics" one-shot**, or show a banner while software drawing is on. Ship it in v0.2.9 with a note.
8. Publish signed checksums (minisign or cosign) and GitHub artifact attestations. Have the updater verify a signature, not a same-origin digest.
9. Credit contributors in the release notes, and say a word when a PR is closed or superseded (#157, #300).
10. Report the `Concat-video-editor` org to GitHub, and choose one canonical domain.
