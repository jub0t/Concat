## host, server, API, CLI, MCP: read at ce64dae (2026-10-20)

Read-only. Nothing built or run. "Verified" means I read it at the line given. "Reasoned" means inferred from code I read. "Unverified" means I could not check it.
Since 10-15: 27 commits touch concat-host, 7 concat-api, 3 concat-server, 4 concat-mcp (new crate), 1 concat-cli. +8 586 / -499 lines across the five crates (`git diff --stat ab21457 HEAD`).
The server step that slipped in both earlier audits has now landed (363800f), and the new MCP crate is a thin adapter of good quality. What is still broken: how claims and the lock interact with agent sessions, a throttle that any local process can turn against the user, and MCP annotations that misstate what tools do.

### Numbers (`#[test]` count, `unsafe ` count, `.unwrap()` count in src/)
| crate | lines | tests | unsafe | unwrap |
|---|---|---|---|---|
| concat-host | 17 541 (+~5 600) | 125 | 46 | 34 |
| concat-server | 2 513 (+1 120) | 27 | 0 | 0 |
| concat-api | 2 971 (+619) | 26 | 0 | 0 |
| concat-mcp | 3 041 (new) | 11 (3 e2e) | 0 | 0 |
| concat-cli | 520 (+3) | 3 | 0 | 0 |
Biggest files: concat-api lib.rs 1 782, playback.rs 1 347, tools.rs 1 133, media.rs 1 109, titles.rs 878, projects.rs 867, updates.rs 862.

---

### Re-verification of every 10-15 item

**10-15 new findings**
| # | Item | Status | Evidence |
|---|---|---|---|
| 1 (M6) | pre-auth fd exhaustion | **FIXED** | registry entry dropped with the connection: `concat-server/src/json.rs:158-169, 292-301`. A seat is taken only after auth (`:275`). Pending callers capped at 32 with a 5 s whole-line deadline (`:241-249, 261, 342-370`). Accept backs off on errors (`:206-224`). Tests `:578, :598, :618` |
| 2 (M13) | crash handler takes locks / allocates | UNCHANGED | `crash.rs` has no commits since 10-15 |
| 3 (M14) | Android second Vulkan device | UNCHANGED | `preview.rs` off_thread untouched (diff is only `forget_plan` `:375-384`). The cards still build a compositor every round (`cards.rs:410`). d9f7e08 only skips software adapters |
| 4 | api lib.rs:7 "an MCP server" | MOOT | MCP now exists (`concat-mcp`) |
| 5 | `connections()` counts subscribers | **FIXED** | counts authenticated seats: `server lib.rs:339-342` |
| 6 (L11) | `ConsolidateClips` adopts any file | UNCHANGED, **wider reach** | `concat-project/src/commands/mod.rs:630`, `clips.rs:670`. An agent can now reach it through MCP `edit_apply` (`tools.rs:241-247`) |
| 7 (L12) | weak tokens, no throttle, 0.0.0.0 | PARTLY | `check_token` (`token.rs:32-46`). The window refuses a weak token (`settings.rs:637`). Throttle added (`throttle.rs`). Off-loopback warning exists (`settings.rs:876`, `is_loopback :1053`). But the estimator passes guessable tokens (new L2), and the throttle is a new DoS (new M1) |
| 8 | two crash codes can never arrive | UNCHANGED | crash.rs untouched |
| 9 | merge bakes un-analysed masks | unverified (window code, not in scope) | — |
| 10/11 | mono output loses right channel; RT-thread frees | UNCHANGED | `playback.rs` no commits |
| 12 (M16) | README promises gRPC + CLI | **FIXED** | `README.md:68` "A build from source adds gRPC and a command-line tool". Releases still build `-p concat` only (`build-app.yml:550`) |
| 13 (L15) | download size unbounded / Windows single rename | size UNCHANGED (`models.rs:224-269`), now reused by sounds (`sounds.rs:432`). Rename **FIXED** | 5 retries with backoff on Windows (`projects.rs:357-372, 388-392`) |
| 14 | merged renders never collected | unverified (window) | — |

**Older items carried by 10-15**
| Item | Status | Evidence |
|---|---|---|
| 12Oct#5 24 held stills FIFO | unchanged (pool not re-read) | `preview.rs:167-170` hold_still as before |
| 12Oct#6 shared save scratch | **FIXED** | per-call `create_new` temporary with pid and nonce (`projects.rs:304-345`). Ticket ordering under a per-folder mutex (`:181-298`). Commits e75af9b, 9197340. The cross-process case is now covered by the lock. The doc at `projects.rs:265-271` still says "a project folder has none [lock] today" |
| 12Oct#30 512 MiB live-title ceiling | UNCHANGED | `titles.rs:98` |
| 12Oct#32a `gpu` capability never appears | UNCHANGED | `Api::with_dirs` uses `Monitor::new()` (`api lib.rs:296`), which sets `gpu: None` (`preview.rs:92-100`), so `has_gpu()` is always false (`:156-165`). `lib.rs:506` never fires. The test pins its absence (`lib.rs:1262`) |
| 12Oct#32b stop waits on open gRPC Events | UNCHANGED (reasoned) | The Events sender lives in the hub subscriber list (`grpc.rs:75-77`). Graceful shutdown waits for the stream (`:170-173`). `close_doors` joins the listeners (`lib.rs:404-406`) before `hub.close()` (`:420`). Only gRPC embedders and tests are affected; the window has no gRPC |
| 12Oct#32c CLI render in place, no MAX_SIDE | UNCHANGED | `concat-cli/src/main.rs:419-431` |
| 12Oct#33 dirs.rs "downloaded models" | **WORSE** | `dirs.rs:32` still says "Large state: downloaded models", but `data` now also holds irreplaceable user content: the audio library and cloned voices (`dirs.rs:89-100`, `sounds.rs`, `voices.rs:75-77`). On Windows that is non-roaming %LOCALAPPDATA% |
| 11Oct#21 docs | PARTLY | api lib.rs:7 now true. `concat-host/Cargo.toml:37` still claims a CPU compositor |
| 11Oct#6 stop hangs on wildcard bind (Windows) | **FIXED** | `loopback()` (`server lib.rs:89-100, 393-395`), test `:515` |
| 11Oct#7 refused thread kills accept loop | **FIXED** | `json.rs:252-256`; errors back off `:218-221` |
| 11Oct#10 API create unchecked | UNCHANGED | `api lib.rs:516-533` calls `projects::create` with no `checked_size`/`checked_rate`. A rate_den of 0 is written, `open` then refuses it (`projects.rs:453`), and the folder is left behind |
| 11Oct#15 server edges | PARTLY | gRPC slow subscriber silently dropped: unchanged (`grpc.rs:75-77`). A poisoned subscriber list silences events: unchanged (`hub.rs:54-57`). CLI `api` dies on non-UTF-8: unchanged (`cli main.rs:175-176`). key_of doc on `impl Drop`: unchanged (`api lib.rs:1064-1066`) |
| 11Oct#16 stop blocks window; jobs die silently | stop **FIXED** (`stop_detached` `server lib.rs:369-379`, used at `settings.rs:624`). Jobs: PARTLY | `reap` books a vanished job as failed (`api lib.rs:897-907`). Under `panic = "abort"` (`src/Cargo.toml:127`) a panic still ends the process. `playback.rs:683, 693` unwraps unchanged |
| 11Oct#17 reads unconfined / caller strings to FFmpeg | UNCHANGED, now documented | `api lib.rs:360-365`. `preset` passes through unvalidated (`:708-711`) |
| 11Oct#23 host lows | UNCHANGED, one **WORSE** | The silent data-folder move (`dirs.rs:124-136`) now also moves `audio`/`voices` user content (`:111-116`). `updates.rs` has no commits. `templates.rs:357` poster as preview.jpg |
| 11Oct#25 WAV header overflow | **FIXED** | `record.rs:621-626` clamps to `u32::MAX - 36`; header `:590-606` |
| 11Oct#29 `--socket` deletes whatever file | UNCHANGED | `server lib.rs:272` |
| S1 unsigned update chain, quarantine strip | UNCHANGED | `updates.rs` has no commits since 10-15. Still this area's worst standing risk |
| S2 API root = whole home | UNCHANGED, wider | `server lib.rs:131-137, 206-210`. MCP headless uses the same default (`concat-mcp/src/backend.rs:185-188`) |
| S3 | **FIXED** | see #1 |
| S4 Clear cache deletes reversed/enhanced media | unverified | `reverse.rs` untouched; `enhance.rs` changed and was not re-read for this |
| S6 embedded API has its own monitor/GPU | UNCHANGED (reasoned) | `api lib.rs:296`. It also shares the process-wide decode pool with the window (`preview.rs:94`), so an agent's previews evict the user's frames |
| S8 HDR proxy name | UNCHANGED | `proxy.rs` untouched |
| S9 composites on hub thread; slow subscribers | hub UNCHANGED (`api lib.rs:840-868`). JSON PARTLY | A full outbox (256) now silently unsubscribes the caller while its connection stays (`json.rs:316, 447-455`, `hub.rs:58-66`). A writer stalled more than 10 s is hung up on (`json.rs:63, 430-437`) |
| S13 per-sample clip scan, S14 silent missing font | UNCHANGED | files untouched |
| S16 connections count | **FIXED** | — |
| S17 Unix socket perms from umask | UNCHANGED | `server lib.rs:273` (no chmod) |

Tally, about 40 items: 11 fixed, 6 partly, 3 worse, about 17 unchanged, the rest unverified or moot. Every 11-Oct and 12-Oct server-step item that was in scope landed in 363800f, and each landed with a test.

---

### New findings

**HIGH: none new.** S1 (the unsigned update chain, unchanged) remains the area's top risk.

**MEDIUM**

**M1. The auth throttle turns loopback into a shared lockout: any local process, or a web page, can lock the user's own agent and scripts out of the window's API indefinitely.** Verified.
- Every TCP caller is keyed by IP alone (`json.rs:93-97`), so all local callers are `Peer::Ip(127.0.0.1)`. Every Unix-socket caller is one `Peer::Local` (`:114-116`).
- Any first line that is not a correct `auth` counts as a failure (`json.rs:267-271`), and that includes `GET / HTTP/1.1`.
- After 3 failures the address is refused before its token is read (`json.rs:228-240`), with waits doubling to 60 s (`throttle.rs:110-116`). The test pins that even the right token is refused (`json.rs:650-659`).
- Failure: a browser tab runs `fetch("http://127.0.0.1:7420")` four times, or any unprivileged local process sends one garbage line a minute. `concat --mcp` is then refused permanently. In `auto` mode it silently falls back to headless (`backend.rs:81-86`, stderr only), which is the path the window mode exists to avoid.
- Second vector: 32 silent sockets renewed every 5 s fill `MAX_PENDING`, and everyone gets `busy` (`json.rs:241-249`). Silent connections are not even throttled (`:262-266`).
- The throttle buys nothing for a minted 128-bit token; the module says so itself (`throttle.rs:6-9`).
- Fix: skip the throttle when the token was minted. Count only well-formed auth attempts. Cap pending connections per peer.

**M2. Projects an agent opens stay claimed and locked until the server stops or the process exits. The window then refuses them with a message that offers no way out, and the agent's unsaved edits are discarded with no warning.** Verified.
- Claims are released only by `project.close` or when the `Api` is dropped (`api lib.rs:579-592, 1066-1073`).
- A connection that ends does nothing to sessions (`json.rs:332-335`).
- The Window backend's `finish` is the no-op default (`backend.rs:47`; `Window` does not override it, `:143-167`).
- A headless `concat --mcp` keeps its `concat.lock` for the whole agent session, because clients keep MCP servers alive.
- Failure: the agent builds "Promo" through the window's server and the session ends. The user clicks Promo in Recents and gets "This project is open through the Remote API" (`en.json:721`, `studio.rs:8169-8171`). The only way to free it is to switch Automation off, and that drops the agent's unsaved edits without a word (the `Api` drop does not save).
- Every keystroke in the host, port or token field restarts the server (`settings.rs:356-372` → `apply_server` → `stop_detached`). Each restart discards every agent session and cancels its export.
- Fix: track sessions per connection and close (or save-prompt) them when it ends. Have `Window::finish` close what it opened. Let the window take over an API-held project with a prompt. Apply settings changes on Enter, not on every keystroke.

**M3. MCP annotations misstate what tools write, and writes reach anywhere under home.** Verified.
- `preview_frame` is `readOnlyHint: true`, `destructiveHint: false` (`tools.rs:445-453`). With `output` it calls `File::create` on any path under the roots, with no extension check and no check for an existing file (`api lib.rs:811-813, 1148-1163`).
- `export_start` claims `destructiveHint: false` (`tools.rs:413-419`) but overwrites an existing `output` (`api lib.rs:665`, muxer guessed from the extension).
- `project_open` is unconfined, and `project_save` then writes `concat.json`, plus the lock and caches, into that folder wherever it is (`api lib.rs:360-365, 600-602`).
- Failure: a client that auto-approves read-only tools (behaviour varies by client, unverified per client) runs an agent that is prompt-injected through title text or media names. Those strings flow into tool output via `summary.rs`. It calls `preview_frame{output:"~/.ssh/config"}` and the file is replaced by a PNG with no prompt.
- `docs/MCP.md` presents "only under your home folder" as a safety property; home is where everything that matters lives.
- Fix: correct the hints. Require `.png` and a new file, or an explicit `overwrite`. Refuse to export over an existing file unless asked. Consider defaulting the roots to Videos/Movies plus the open project folders.

**M4. A lock that fails with an I/O error lets the open go ahead with no guard.** Path verified; Windows behaviour reasoned.
- `OpenProjects::claim` maps `LockError::Io` to "no lock, open anyway" (`api lib.rs:183-189`).
- `acquire` turns `PermissionDenied` into `Io` after 4×50 ms (`lock.rs:97-106`).
- On Windows a lock file held without sharing by an AV scanner, OneDrive or Search, or left delete-pending on volumes without POSIX delete semantics (FAT/exFAT, SMB, older builds; unverified which builds), gives exactly that error.
- Windows `same_file` is only `path.exists()` (`lock.rs:257-263`), so a lock taken on a delete-pending file passes the check. Every later opener then gets `PermissionDenied` → `Io` → an unguarded open. This is two processes saving one folder, the thing 68ceffe set out to prevent, on the priority platform.
- Network file systems with no locks degrade silently too.
- Fix: refuse, or open read-only, when the folder is writable but cannot be locked. Retry `PermissionDenied` for longer.

**M5. `concat --mcp` as shipped is never run by CI, and its panic safety net is dead code.** Verified.
- The shipped profile is `panic = "abort"` (`src/Cargo.toml:127`). That makes `catch_unwind` in `protocol.rs:426-429` ("a panic in a tool is that call's failure") and `hub.rs:91-104` (inside the window's server) no-ops.
- Any engine panic kills the agent session and its running export. Inside the window it kills the window and the user's unsaved work.
- The release binary is GUI-subsystem on Windows (`concat/src/main.rs:8`); its use as a stdio server is untested.
- The e2e test drives the dev `concat-mcp` binary (`tests/e2e.rs:29`). The workflows contain no `--mcp` (grep of `.github/workflows`).
- The discovery → `Window::find` path has no end-to-end test either: e2e sets `CONCAT_API_ADDRESS`/`TOKEN` (`e2e.rs:385-391`).
- Fix: a CI smoke test that runs `concat --mcp headless` from the app-profile build on Windows and macOS, sending `initialize`, `tools/list` and one edit.

**LOW**
- **L1.** The Window backend re-sends any request once after a connection error (`backend.rs:144-162`). If the server applied it and the reply was lost (server restart mid-call, or the 10 s write-timeout hang-up `json.rs:430-437`), a `clip_add` or `captions_add` lands twice. The hang-up is reachable: the client reads events only during a call (`client.rs:95-115`), so an idle agent during an export backs up the 256-line outbox. Retry only idempotent tools.
- **L2.** The token estimator (`token.rs:53-72`, floor 64) passes `12345678901234567890` (about 66 bits) and `abcdefghijklmnopqrst` (about 86). On a 0.0.0.0 bind, IPv6 address rotation evicts strikes once 4 096 peers are booked (`throttle.rs:76-89`). Any success from a shared IP wipes its strikes (`json.rs:273`, `grpc.rs:204`).
- **L3.** Inline previews are unbounded. `width`/`height` up to 8192 (`tools.rs:878-904` defaults only when both are absent) gives a ~256 MB RGBA frame, PNG-encoded and base64'd into one JSON line headed for the agent's context. Above the client's 256 MiB `MAX_REPLY` (`client.rs:21`) the line is cut, read as "not JSON-RPC", and retried through L1, so the frame renders again.
- **L4.** Recents are written non-atomically (`projects.rs:545-553`) by the window, the window's API and every headless MCP process (`api lib.rs:570-572`). A read during a write returns empty, and the next `remember` wipes the list. Agent projects also fill the user's Recents and home page.
- **L5.** `Api::cancel` cancels the shared export slot (`api lib.rs:786-789`), not its job's flag, contrary to `job.rs:13-16`'s own rationale. A small race can cancel the window's export.
- **L6.** Job names restart at `j1` for each `Api` (`api lib.rs:892-895`). After a server restart, an agent polling `j1` gets "no job" or someone else's job.
- **L7.** The Tripwire decides by pid liveness (`jobs.rs:157-165`), while `lock.rs:18-23` explains why pids answer wrongly. On Windows and Android a reused pid lets a crash-loop job restart itself.
- **L8.** No read timeout after auth (`client.rs:55-59`). With one request at a time (`protocol.rs:18-22`), a stuck hub or a 600 s `export_wait` blocks every other tool, `export_cancel` included.
- **L9.** Two windows: the second overwrites `automation.json`, and when its server stops it removes the listing (`discovery.rs:134-141`), so the first window's running server is no longer listed.
- **L10.** Sound downloads trust a digest from the same manifest as the URL, with no size cap (`sounds.rs:383-395, 432-447`; `models.rs:224-269`). Same class as S1.
- **L11.** Stale or misplaced docs:
  - `prefs.rs:230-231` says an empty token means "none"; it means a minted one.
  - `projects.rs:265-271` says there is no lock.
  - `settings.rs:964-969`: `new_token`'s doc sits on `LOG_LEVELS`.
  - `settings.rs:1044-1052`: `split_listen`'s doc is merged into `is_loopback`'s.
  - `api lib.rs:1064-1066`: key_of's doc sits on `impl Drop`.
  - `protocol.rs:426` is false under abort.
  - `concat-host/Cargo.toml:37` still claims a CPU compositor.
  - `dirs.rs:32` "downloaded models".
  - `is_exposed` (`server lib.rs:77-82`) is unused; the window duplicates it as `is_loopback`.
- **L12.** The MCP "modern" protocol `2026-07-28` with `server/discover` (`protocol.rs:36-41`) is past my knowledge: **unverified** against the spec.

### Commits since 10-15, area view
- **363800f server hardening.** Correct in shape: RAII seats and places, a registry that cleans itself up, a whole-line auth deadline, a bounded outbox with a write timeout, and stop cancelling before joining. Each guard has a test (`json.rs:545-778`). What it gets wrong is the throttle model (M1).
- **56ff055 discovery + client.** Owner-only 0600 write via staging and rename (`discovery.rs:76-110`); removal checks the pid. Fine. Windows relies on the folder's ACL (unverified).
- **db529b2 / f70332c / 30584f7 MCP.** A thin adapter, with schemas generated from the API types and round-trip tested (`tools.rs:1087-1116`). Cancellation and progress are handled properly. Annotations wrong (M3); shipped path untested (M5).
- **68ceffe / 46e04b6 project lock.** An OS lock rather than a pid file, takeover after a crash, an inode recheck against the release race (`lock.rs:113-118`), a real child-process test (`:300-345`). Holes: M4, and M2's lifetime problem.
- **e75af9b save ordering.** Fixes 12Oct#6 properly.
- **Host features** (sounds, voices, stabilize, enhance preview, tripwire, voiceover recorder): skimmed for path and security handling only. File names are sanitised (`sounds.rs:162-168, 224-252`; `voices.rs:89-99`) and the voice decode is bounded (`voices.rs:117-120`). Not audited for behaviour.

### Good
- The fd-leak fix is complete and tested.
- Stop is detached from the UI thread.
- The lock is designed properly.
- Saves are ordered, with their own temporaries and Windows retries.
- MCP schemas come from the types, so they cannot drift.
- Stdout carries protocol only: no `println!` in the engine crates outside tests (grep).
- The discovery file is owner-only.

### Scores (area; 10-15 is the mean of the host, server, API and CLI rows)
| Aspect | 10-20 | 10-15 | Why |
|---|---|---|---|
| Code quality | 8 | ~7.3 ↑ | The server rewrite is careful RAII with no unwraps. Dead `catch_unwind` and misplaced docs remain |
| Design architecture | 7 | ~7.3 → | One dispatcher with thin transports, MCP as an adapter: right. Claims are not tied to connections (M2), the lock fails open (M4), the embedded API has its own monitor and the shared pool |
| Design philosophy | 8 | ~6 ↑ | "Refuse rather than lose", an OS lock rather than pids, schemas from types. The Tripwire contradicts it (L7) |
| Maintainability | 7 | ~7.3 → | Clear modules, but `api lib.rs` is 1 782 lines and `tools.rs` 1 133; docs drift (L11) |
| User likeability | 6 | (Remote API 4) ↑ | MCP is real value with a one-line setup. Dead-end refusals (M2), Recents pollution (L4), settings edits that kill agent work |
| Performance | 6 | ~5.7 → | One hub thread composites synchronously; previews unbounded (L3); an agent shares the window's decode pool |
| Scalability | 5 | 5 → | One API thread, one export slot, 64 seats: fine for a local tool, nothing more |
| Security | 5 | ~4.5 ↑ | Pre-auth fd leak fixed, token floor, owner-only discovery. A new lockout DoS (M1), annotations that understate writes plus home-wide roots (M3), and S1 untouched |
| Testing | 7 | ~5.5 ↑ | 27 server tests, a cross-process lock test, MCP e2e headless and via server, schema round trips. The shipped `--mcp` path, the discovery path and Windows lock races are untested (M5, M4) |
| Documentation | 6 | 6 → | `docs/MCP.md` is clear and accurate on modes, but overstates the safety of "home". About ten stale or misplaced comments (L11) |

Top fixes, in order: M1 (no throttle on a minted token), M2 (claims tied to sessions), M3 (correct hints and refuse overwrites), M5 (CI smoke of `concat --mcp` on Windows), M4 (fail closed), then S1.
