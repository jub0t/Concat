## host, server, API, MCP, CLI, project, speech, vision, and security across the codebase: read at 2606aff1 (2026-10-24)

Read-only. Nothing built or run (another agent holds the shared target dir). "Verified" means I read it at the line given in the current tree. "Reasoned" means inferred from code I read. "Unverified" means I could not check it. Outside checks: `gh` (read-only), the Hugging Face and Gradle checksum endpoints, the upstream whisper.cpp commit through the GitHub API.

Baseline is c41865e (20 Oct). Since then, 102 commits. In this area most of the diff is the copyright-header rename. Real changes: concat-server `json.rs` +76 and `lib.rs` +53 (the #431 stop), concat-api only `message.rs` docs, concat-mcp `protocol.rs`/`tools.rs` (+178), concat-cli and concat-vision headers only, concat-speech `transcribe.rs` +132 and an 885 KB model, concat-host `net.rs` (new, 82), `projects.rs` +40, concat-project +1 000 (speed curve, MatchClip, sound layers), concat-mobile (new, 1 321 lines), `android-native/` (new Gradle spike).

Short version: two things got better. Prefs are now atomic and 0600, and captions are timed from Silero VAD plus the upstream whisper fix. One thing got worse: the updater now trusts any root certificate in the system store, so the one barrier it had, Mozilla's root list, is gone. Every server and MCP finding from 20 Oct is untouched.

### Numbers (`#[test]` count, `unsafe` count, `.unwrap()` count, all `.rs`)
| crate | lines | tests | unsafe | unwrap |
|---|---|---|---|---|
| concat-host | 18 547 (+~1 000) | 137 (+12) | 46 | 61 |
| concat-server | 2 598 (+85) | 28 (+1) | 0 | 0 |
| concat-api | 2 973 | 26 | 0 | 0 |
| concat-mcp | 3 199 (+158) | 13 (+2) | 0 | 0 |
| concat-cli | 520 | 3 | 0 | 0 |
| concat-project | 13 777 (+878) | 165 (+15) | 0 | 41 |
| concat-speech | 3 635 (+131) | 24 (+2, 1 ignored) | 5 | 10 |
| concat-vision | 5 498 | 59 | 0 | 5 |
| concat-mobile | 1 321 (new) | 2 | 10 | 0 |

---

### 1. Status of the 20 Oct findings in this area

| 20 Oct item | Status | Evidence (current tree) |
|---|---|---|
| H5 / S1: updater checks a digest from the same release | **WORSE** | Still no signature: `updates.rs:375-414` reads `sha256` from `manifest.json`, which `manifest_url` (`:416-418`) fetches from the same release, and `download_package` (`:440-473`) checks against it. `release.yml:151` writes the manifest unsigned, with no attestation. Quarantine is still stripped (`updates.rs:584-586`). New: 46af73c sends `get_text` (`:671`) and the package download through `net::agent` (`net.rs:23-29`), which trusts the system root store as well (`net.rs:56-60`). See H1 |
| M1: shared auth-throttle bucket | UNCHANGED | Every TCP peer is keyed by IP (`json.rs:95`), so all local callers share one key. Any bad first line counts as a failure (`:269`, `handshake :405-411`, which includes `GET / HTTP/1.1`). Refusal comes before the token is read (`:229`). `throttle.rs:6-7` still says a minted token needs no throttle, and the code throttles it anyway |
| M2: agent-opened projects stay locked | UNCHANGED | A connection end only unsubscribes (`json.rs:343`). `Window::finish` is still the no-op default (`concat-mcp/src/backend.rs:47`). Claims are released only by `close` or by `Drop for Api` (`api lib.rs:578-591, 1067-1074`) |
| M3: MCP hints understate writes | UNCHANGED | `preview_frame` is `read_only: true, destructive: false` (`tools.rs:462-468`), yet with `output` it `File::create`s any path under the roots (`api lib.rs:811, 1148-1160`). `export_start` is not marked destructive and overwrites (`:665`). `docs/MCP.md:47` still sells "only under your home folder" as the safety property |
| M4: lock opens unguarded on I/O error | UNCHANGED | `api lib.rs:186-188` (`LockError::Io(_) => None`). Windows `same_file` is still `path.exists()` (`lock.rs:261-262`) |
| M5: shipped `concat --mcp` never run in CI; dead `catch_unwind` | UNCHANGED | No `--mcp` in `.github/` or `scripts/ci`. `panic = "abort"` (`src/Cargo.toml:127`); the comment at `:124-126` now admits it |
| Prefs world-readable / non-atomic | **FIXED** | 4b1b2b05. Staged `settings.json.<pid>.part` → rename (`concat/src/prefs.rs:352-363`). Created 0600 on Unix (`:391-401`), test `:453-457`. A bad value now costs only itself (`salvage :368-387`), and the original is kept as `.unreadable` (`:337`). Small hole: a stale `.part` left by a crash keeps its old mode, because `mode()` applies only on create (Low) |
| Downloads without size caps | UNCHANGED | `models.rs:224-305` writes until EOF with no cap against `estimate`. Sounds use the same path |
| Phone FFmpeg / sherpa pulled unverified | UNCHANGED | `src/scripts/ffmpeg-mobile.sh:39`, `sherpa-mobile.sh:42, 60`: `curl` with no digest. Desktop is digest-pinned (`scripts/ci/fetch.sh:66-79`) |
| Model mirror 404 | UNCHANGED | `gh api .../releases/tags/models-v1` returns 404 today, and `models.yml` has no runs. `THIRD_PARTY_NOTICES.md:24-26` still says Concat "mirrors them and downloads them on demand from that mirror" |
| H8: captions start early (#438) | **FIXED in code**, unverified on audio | Two layers. The upstream whisper.cpp parser fix (`patches/whisper-rs-sys/.../whisper.cpp:7659-7687`, PR #528). Silero VAD spans clamp every segment (`transcribe.rs:237-300, 602, 625`). Only `on_speech` is unit-tested (synthetic spans). No test runs the bundled VAD on audio, though it ships in-tree and needs no download. #438 is still open, correctly, with no reporter confirmation |
| L1 Window backend retries non-idempotent calls | UNCHANGED | `backend.rs` header-only diff |
| L2 token estimator / IPv6 eviction | UNCHANGED | `token.rs:18, 53-72` |
| L4 Recents non-atomic | UNCHANGED | `projects.rs:545-553` (`fs::write`) |
| L10 sounds trust a same-manifest digest | UNCHANGED, now wider (H1) | `sounds.rs:382-395` |
| L11 doc drift | PARTLY | `THIRD_PARTY_NOTICES.md:6` still lists libavdevice, which af907228 dropped. `dirs.rs`, `protocol.rs`, `prefs.rs` and the other L11 comments are unchanged apart from headers |
| S17 Unix socket perms from umask | UNCHANGED | `server lib.rs:272-275` |
| 11Oct#10 API create unchecked | UNCHANGED | no diff in `api lib.rs` beyond the header |
| N6 (project): transition params bypass NaN guard | **FIXED** | `commands/mod.rs:1136` now checks `tr.params` |

Tally, 20 items: 3 fixed, 1 partly, 1 worse, 15 unchanged. The server, API and MCP list got no work since 20 Oct.

---

### 2. New findings

**HIGH**

**H1. The updater now installs whatever a TLS interceptor serves.** Verified (code); unreleased (46af73c, after v0.2.8).
- `net::agent` trusts webpki-roots **plus** every root in the OS store (`net.rs:52-62`). rustls-native-certs also honours `SSL_CERT_FILE` and `SSL_CERT_DIR`.
- The updater has no signature. The digest it checks comes from a manifest fetched over the same connection (`updates.rs:416-473`). So before this commit, the only thing standing between a network attacker and code execution was "the certificate must chain to Mozilla's list". That is now "to any root this machine trusts".
- The commit's own motivating case is exactly the attacker: "a proxy or a GitHub accelerator in China, an antivirus that scans HTTPS, a company's gateway" (`net.rs:9-13`). Each of these re-signs github.com and can now swap `manifest.json` and the package. The updater then runs `setup.exe` / `msiexec` with no Authenticode check (`updates.rs:489-505`), or swaps the `.app` and strips quarantine (`:584`).
- Models are fine: their digests are compiled in (`models.rs:311-330`). Sounds are not: digest and file come from one fetched manifest (L10), so an interceptor can feed FFmpeg's parsers arbitrary audio.
- Fix before the next release, in order of cost:
  1. Give `updates.rs` (and `sounds.rs`) a webpki-only agent. This restores the 0.2.8 posture; it is one function.
  2. Sign the manifest with an ed25519/minisign key whose public half is compiled into the app, and check the signature before reading the digest.
  3. On Windows, check WinVerifyTrust on the installer once signing exists.
- Keep system roots for model downloads only. That is where #517's users actually failed, and the digests make the wider trust harmless there.

**MEDIUM**

**M6. A cross-project paste still carries the other project's media ids, so Enhance-off and image backgrounds play the wrong file.** Verified.
- 1d04d147 remaps `clip.media_id` by path (`studio.rs:8288-8320`). `MatchClip` then copies the rest of the source clip wholesale (`commands/properties.rs:330-360`). That includes `enhanced { original, copy }` (`model.rs:1310-1317`) and `background: Image { media_id }` (`model.rs:731-735`), and both are bin ids of the *source* project.
- Failure: copy an enhanced clip in project A and paste it in B. Toggle Enhance off. `SetClipEnhanced` sets `media_id = link.original` (`commands/clips.rs:498-515`), which is A's `m3`. In B that is another file or none. An image background draws B's `m4` or nothing.
- The commit message says "a paste into another project plays the right file", which is true only until either of these is touched.
- Fix: in `held_here`, remap `enhanced.original`/`copy` and `Background::Image` by path the same way, or clear them when the target has no such file. Add a test with both fields.

**M7. The VAD step runs blind: no cancel, no progress, a fixed-name `.part`.** Verified; timings reasoned.
- `speech_spans` runs over the whole window before `state.full` (`transcribe.rs:602`). Whisper's abort callback is not installed for it, and progress sits at the first tick. On an hour of audio Silero takes on the order of tens of seconds on CPU (unmeasured), and Cancel does nothing until whisper starts.
- `vad_model` writes `ggml-silero-v6.2.0.part` (`path.with_extension("part")`, `:288-300`), the same name in every process. Two windows captioning at once truncate each other's file, and one of them may load a half-written model. That run falls back to whisper's times with only a log line.
- "Whole" is decided by length alone (`:290-291`).
- Fix: check `cancel` around the VAD call (or run it on the abort hook). Use a pid/nonce `.part` as `prefs.rs:360` now does. Compare the bytes once, or a digest, rather than the length.

**M8. Forward compatibility: a project with a sound layer loses it on the first autosave in 0.2.8.** Verified against `v0.2.8:model.rs:1652-1655`.
- 0.2.8 reads the unknown kind `"soundLayer"` as `Video` (`wire::clip_kind`). `settle_clip` then drops it, because a Video with an empty `media_id` has no media (`doc.rs:264-272`). The next save writes the project without it. `DOCUMENT_VERSION` stays 1 (`doc.rs:41`).
- The model doc (`model.rs:67-77`) calls this "leaves it out as a clip it cannot play". That holds until the autosave.
- Phones lag desktop releases, and a project synced between a phone on 0.2.8 and a desktop on main is the realistic case. Same class as 20 Oct N7, now losing whole clips.
- Fix: bump a minor document marker that older builds refuse to *save* over (open read-only), or carry unknown clips through as opaque `extra`.

**LOW**
- **L13.** On Windows, cross-project paste looks a file up by exact path (`studio.rs:8297`), while `AddMedia` dedups case- and slash-insensitively (`commands/media.rs:11-27, 37-41`). Take `C:\v\a.mp4` against `c:/v/A.mp4`: `held_here` finds nothing, `AddMedia` returns `Outcome::default()`, `apply_in` gives `None`, and that clip is silently not pasted. Use `same_path` in both places.
- **L14.** concat-mobile (a spike, not shipped). `Engine::attach_surface(window: u64)` is a safe UniFFI export that turns a Kotlin integer into `NativeWindow::from_ptr` (`render.rs:469-474`): an unsound boundary, with only null checked. `Engine::new` calls `set_var` (`lib.rs:141-145`) under a SAFETY note that fails on a second construction, because the first engine's threads are running. `ANativeWindow_fromSurface`'s reference leaks if `attach_surface` is never called (`jni.rs:47-55`). `gradle-wrapper.properties` has no `distributionSha256Sum`; the committed wrapper jar does match Gradle's published 8.12 checksum (verified).
- **L15.** `platform.rs:451-454` calls `set_var("CONCAT_GPU")` after the GPU probe, under a SAFETY claim that the process is single-threaded. On Linux, llvmpipe and other Mesa drivers have started threads by then (reasoned). This is a Unix setenv race; Windows is unaffected.
- **L16.** SVG import (f5664c83, `concat-media/src/svg.rs`) uses `usvg::Options::default()`, so an SVG's `<image href="/abs/path">` is read from disk and drawn into the still (reasoned from usvg's default resolver). A downloaded SVG can embed a local picture into an export the user later shares. Set `resources_dir: None` with a resolver that refuses file hrefs, or allow `data:` only.
- **L17.** `concat-speech/Cargo.toml:18` still depends on `ureq`, which nothing in the crate uses. `THIRD_PARTY_NOTICES.md` names the MIT licences of whisper.cpp and Silero but reproduces neither text, which MIT's notice condition asks for in binary distributions.
- **L18.** The MCP stage names are string literals shared across three crates (`concat-export lib.rs:1250-1257, 1765, 2000`, `concat-mcp tools.rs:43-52`, `api message.rs:429-433`). A rename in the exporter silently drops progress to stage 0. Export a constant set from concat-export.
- **L19.** `ci.yml`/`mobile.yml`: dependabot moved `dtolnay/rust-toolchain` to 683b588 but the comment still reads "2026-10-09". Workflows other than `models.yml`, `nix.yml` and `release.yml` still have no `permissions:` block (20 Oct L2).
- **L20.** The MCP line reader splits a line over 16 MiB into chunks, each parsed on its own (`protocol.rs:44, 155`). This input comes from the client, so it is not a security issue, but the tail of a long line can run as a request. Discard to the newline instead.

---

### 3. Commits since c41865e, area view (independent review)

- **2922614c VAD (#438).** Correct in shape.
  - The bundled model is the real one: sha256 `2aa269b7…6987` matches Hugging Face's `x-linked-etag` for `ggml-org/whisper-vad/ggml-silero-v6.2.0.bin` (verified), and the size, 885 098 bytes, matches.
  - Centisecond units are right: whisper-rs 0.16 `whisper_vad.rs:261-301` documents `start`/`end` in centiseconds.
  - `on_speech` cannot invert a span for sorted, valid spans, and falls back to whisper's times when nothing overlaps.
  - `include_bytes!` links only into `concat` (the one crate that depends on concat-speech).
  - Holes: M7, plus the missing audio-level test.
  - Notices entry present (`THIRD_PARTY_NOTICES.md:29-33`).
- **ea225312 / f1a5f4d3 whisper.cpp patch (#528).** Byte-identical to upstream c33c5618 (whisper.cpp #2279, 2026-05-10, after 1.8.3), compared through the GitHub API. Documented in `patches/README.md:67-88`, including a note to drop it on upgrade. Good contributor work.
- **46af73c TLS.** Well reasoned for downloads, and the shipped roots are always kept (test `net.rs:72-81`). It applies one trust policy to code and to data alike (H1). No duplicate TLS stack (one rustls 0.23.45, ring 0.17.14; `Cargo.lock`).
- **1d04d147 MatchClip.**
  - The right idea: copy the whole clip, keep only the place. `#[serde(skip)]` keeps it off the wire, and the test proves `{"op":"matchClip"}` is refused (`lib.rs:617-618`).
  - Nothing serializes a `Command` outside a test (grep), so a skipped variant cannot panic a serializer.
  - Holes: M6 and L13.
- **2606aff1 sound layer.**
  - The serde default `sound: false` keeps old API callers working, and old projects read unchanged.
  - The export chain is kind-filtered: a picture package on a sound layer yields an empty chain, and `audio_buses` (`concat-export lib.rs:1353-1370`) drops it. Strength is clamped to 0-1.
  - The MCP description of `effect_layer_add` is updated (`tools.rs:391`).
  - Hole: M8.
- **77ce50bb concat-mobile spike.** Narrow API, every unsafe has a SAFETY note, and it is not built by any workflow (grep), so it ships to nobody. L14.
- **4ffa3212 / server stop (#431).** The stop now cancels jobs, joins the API thread, then drains each outbox before hanging up (`server lib.rs:364-370`, `json.rs:298-311`), with a test (`json.rs:796-843`). The drain is sequential across connections, up to 10 s each, but the window bounds it with `EXPORT_PATIENCE` on its own thread (`concat/src/lib.rs:2598-2611`). Fine.
- **3dd2ee95 / 1a2c71ff / 9f465da4 MCP.** Non-UTF-8 lines answered and the session kept (test `protocol.rs:523-534`). Stage mapping (L18). `ttlMs`/`cacheScope` on `server/discover` and `tools/list` per the 2026-07-28 draft: still **unverified** against the spec (past my knowledge, as on 20 Oct).
- **e737cf89 device names.** Correct for CON/PRN/AUX/NUL/COM0-9/LPT0-9 and the superscript ports; it misses `CONIN$`/`CONOUT$` (trivial).
- **4b1b2b05 prefs.** See §1.
- **f312433c CI.** fmt, locales and models are now in their own job, with `if: !cancelled()` (`ci.yml:35-66`). This answers the 20 Oct "gates hide clippy" finding.

### 4. Security pass

| Surface | Finding |
|---|---|
| Port 7420 from a web page | A page cannot drive the API: the first line must be a JSON `auth` (`json.rs:398-422`), and a browser's first line is `GET`/`POST … HTTP/1.1`. CORS and Origin do not apply (not HTTP), and DNS rebinding gains nothing without the token. What a page *can* do is M1: four `fetch()`es lock every local caller out for up to 60 s per strike, indefinitely. Unchanged |
| Tokens | Not logged anywhere (grep of `log::*!` in server, MCP, settings). Discovery file 0600 (20 Oct); prefs now 0600 (fixed). Windows relies on profile ACLs (unverified) |
| Path traversal | Writes are confined by `writable()`: `..` refused, symlinks resolved on the existing prefix (`api lib.rs:379-412, 1108-1130`). Opens are unconfined by design, and `project_open` + `project_save` still write `concat.json` and `concat.lock` into any existing project folder (M3). Package and sound names are sanitised (`updates.rs:396-401`, `sounds.rs`), and `folder_name` now also escapes device names |
| Updater | H1 (worse than 20 Oct): no signature, no attestation, same-release digest, system roots trusted, quarantine stripped |
| Unsafe added since 20 Oct | `concat-media decode.rs` seek_top and `ffi.rs` sample_aspect: sound (plain field reads on open handles). concat-mobile: 10 (L14). `platform.rs:454` `set_var` (L15). No new unsafe in my crates outside the spike |
| Secrets in repo | None. Grep for AWS, GitHub, Slack, HF, OpenAI and Google key shapes and private-key headers across tracked files came back empty, and there are no keystores or `.env` files |
| Supply chain | Desktop prebuilts digest-pinned (`deps.lock`, `fetch.sh`). Phone FFmpeg and sherpa unpinned (unchanged). Actions pinned by SHA. Gradle wrapper jar genuine but distribution unpinned (spike). Homebrew on macOS builds unpinned (20 Oct, not re-checked). Silero model verified against upstream |

### 5. Scores (this area)
| Aspect | 10-20 | Now | One line of evidence |
|---|---|---|---|
| Code quality | 8 | 7 ↓ | Careful new code with tests (stop drain, NUL names, prefs salvage), but MatchClip copies project-local ids (M6) and the VAD `.part` repeats the shared-scratch mistake fixed in saves on 12 Oct (M7) |
| Design architecture | 7 | 6 ↓ | `net.rs` as the single network choke point is right, but it gives code and data one trust policy (H1). Claims are still not tied to sessions (M2), the lock still fails open (M4), and a new clip kind drops on downgrade (M8) |
| Security | 5 | 4 ↓ | Prefs fixed (+). Updater trust widened (H1). M1, M3, M4, the size caps and the phone deps all unchanged |
| Testing | 7 | 6 ↓ | Every commit carries a test, but the VAD is never run on audio although it ships in-tree, there is no `--mcp` smoke, no cross-project paste test with enhance or background, and the TLS test only checks that a config builds |
| Documentation (API/MCP) | 6 | 6 → | `patches/README.md` and the stage docs are exemplary. `MCP.md:47` still oversells "home", `THIRD_PARTY_NOTICES.md` claims a mirror that 404s and a libavdevice that is gone, and the model doc calls M8 safe |

### 6. Before the next release
1. **H1**: a webpki-only agent for `updates.rs` and `sounds.rs` (an hour). Then a signed manifest.
2. **M6**: remap or clear `enhanced` and `Background::Image` in `held_here`, with a test.
3. **M3**: `preview_frame` → `readOnlyHint: false`, `destructiveHint: true`, and refuse to overwrite without a flag. One-line hint fix; the overwrite guard is a few lines.
4. **M7**: cancel check and a unique `.part` for the VAD model.
5. Then the standing server items in the 20 Oct order: M1 (no throttle for minted tokens), M2 (sessions per connection), M5 (CI smoke of `concat --mcp`), M4 (fail closed). After those, the size caps, the phone digests, and running `models.yml` once.
