## host, server, api, cli: reviewer 2, read at ab21457 (main moved to 2e4c4d5 during; only 8 lines of concat-host/tests/export.rs touched)
Verified = read at line; reasoned/suspected otherwise. No cargo run.

### Re-verified
- 12Oct#5 pool 24 stills FIFO, forget each pass: UNCHANGED pool.rs:541, 628-652, 675-685; hold_still now concat-host/src/preview.rs:169.
- 12Oct#6 shared save scratch: UNCHANGED, worse outcome silent. projects.rs:174 fixed concat.json.saving; studio.rs:2633-2650 saves on 3-worker pool, dirty=false (:2638) before write; two renames land either order -> older doc on disk, no error, no dirty flag. Close saves on UI thread (:7300-7304). (reasoned)
- 12Oct#30 live-title ceiling constant: titles.rs:98 512 MiB.
- 12Oct#32 api/server/cli smalls: gpu capability never appears (concat-api/src/lib.rs:180, 370-372, test pins absence); stop waits on open gRPC Events (grpc.rs:162-170, lib.rs:281-296); CLI render in place, no MAX_SIDE (concat-cli/src/main.rs:419-431).
- 12Oct#33 dirs.rs:32 "downloaded models": unchanged.
- 11Oct#21 MCP/CPU fallback docs: PARTLY. README fixed; concat-api/src/lib.rs:7 still "an MCP server"; concat-host/Cargo.toml:37-38 CPU compositor claim.
- 11Oct#6 Server::stop hangs on wildcard bind on Windows: UNCHANGED concat-server/src/lib.rs:270-272, 281-283; on UI thread settings.rs:557-558.
- 11Oct#7 OS-refused thread kills accept loop: UNCHANGED json.rs:128, 183, :305. panic=abort in [profile.app] -> aborts whole app.
- 11Oct#10 API writes unchecked: export checks only roots (api lib.rs:520); create no size/rate check (:380-397); projects::create writes rate_den 0 (projects.rs:106-148) which open refuses (:229-231), folder left.
- 11Oct#15 server edges: grpc.rs:74-76; poisoned subscriber list silences events (hub.rs:54-57); hub.rs:155-157; CLI api ends on non-UTF-8 line (main.rs:175-176); doc on impl Drop (api lib.rs:868-870), key_of (:933) none.
- 11Oct#16 stop blocks window; jobs/workers die silently: lib.rs:293-296 waits api.finish() joining export (api lib.rs:748-752); catch_unwind (hub.rs:91-104) useless under abort; ExportJob::run (:761-786) no catch -> no export.failed; playback.rs:683, 693 .unwrap() on locks.
- 11Oct#17 reads unconfined; caller strings reach FFmpeg: api lib.rs:226-230, 326-328, 465-469.
- 11Oct#23 host lows: silent data-folder move dirs.rs:108-119; any Program Files program = MSI install updates.rs:286-302, case-sensitive starts_with; templates.rs:356 poster as preview.jpg.
- 11Oct#25 WAV header overflow record.rs:287, 315.
- 11Oct#29 --socket deletes whatever file: concat-server/src/lib.rs:195.
- S1 update trusts digest beside package: digest from same manifest as URL (updates.rs:416-428, 465), no signature; setup.exe no Authenticode (:491-495); hdiutil -noverify (:539); xattr -dr quarantine (:584-586) bypasses Gatekeeper; any release >=0.2.5 installable incl. downgrades.
- S2 API root is home: concat-server/src/lib.rs:80-86.
- S3 fd per connection, seats before auth, accept spins: UNCHANGED, worse than scored (finding 1).
- S4 Clear cache deletes reversed/enhanced media: reverse.rs:88, enhance.rs:71, projects.rs:341-350.
- S6 embedded API own GPU device: api lib.rs:179; Vulkan window export too now.
- S8 HDR proxy name/contents disagree: unchanged.
- S9 API composites on hub thread; slow subscribers dropped silently: api lib.rs:678-706; json.rs:319-326; grpc.rs:74-76.
- S13 mixer scans every clip per sample: playback.rs:1139-1166.
- S14 missing font silent: titles.rs:441.
- S16/S17: connections() counts subscribers (lib.rs:257-259); Unix socket umask perms (:196).
Tally ~25: 0 fixed, 1 partly, 1 escalated, rest unchanged. None of the 11 Oct "server step" items landed.

### Commits
- 14b5998: does it. off_thread (preview.rs:146-153) None on Vulkan; export (panes/export.rs:518), cards (studio.rs:7481), merge (studio/consolidate.rs:116); look trials stay sibling (studio.rs:7543). Diagnosis right (Skia submits to raw VkQueue without wgpu lock). Misses: Android also Vulkan (concat/src/gpu.rs:52-57), commit titled linux doesn't say; no test. Refs #324.
- 8f5b141 crash line + machine header (#326): Good: one line with code, thread, module+offset; chains to previous handler/filter; SA_ONSTACK, CRASHING guard, dup'd fd; installed before --version (main.rs:19), open_logging (lib.rs:106), CLI (main.rs:128); child-process test passed Linux/macOS/windows-2022 (run 38435821575, job 115429378305); machine.rs writes build, commit, OS, CPU name+flags. Problems: dladdr (crash.rs:217) takes loader lock (glibc dl_load_lock, bionic g_dl_mutex, dyld): blocks if another thread holds it; SAFETY "only reads loader's tables" (:213-214) false. to_string_lossy (:218) can allocate. Windows: std::thread::current() (:398) allocates on non-Rust threads (ORT pool, drivers, FFmpeg) and panics after TLS teardown -> fail-fast abort; GetModuleFileNameW (:442) loader lock. ~3 KB buffers on faulting stack, no alt stack on Windows (suspected stack-overflow loss). Crash during crash: Unix correct; Windows nested exception loses line (write after describe :379-382). Handles dup'd once, CLOEXEC/non-inheritable, swapped atomically; keep closes old fd (:137-141) race only if log opened twice. Handler re-pointed before logger install can fail (logs.rs:78 vs :89). Untested: write to log file; crash on non-Rust thread.
- fc7e6a9 consolidate (host): consolidate.rs 219 lines captures clips/media/settings, rechecks before adopting (:52-68); normal exporter with effect_time_offset; pads to whole frame; window deletes unadopted render; 2 unit + 2 end-to-end tests (+284 tests/export.rs) render, probe, adopt, export again, undo, redo, cancel: best-tested work in period. Misses: API exposes raw command (6); CRF16 "fast" hard-coded, HDR -> 10-bit HEVC hardware-first (consolidate.rs:120-132, NVENC on Optimus); doesn't wait for cutout masks (9); adopted-then-undone renders never collected (14); no Refs.

### New findings
HIGH: none.
MEDIUM
1. Any process can exhaust the window's fds via Remote API without a token (verified). Every TCP/Unix connection leaves a dup'd socket in connections (json.rs:109-122, 164-177) until stop; seat before handshake (:102, :157). A few hundred connections (256 macOS, 1024 Linux): saves fail "Too many open files" (projects.rs:186-187), decoders fail, accept loop spins 100% CPU (:100, :156). A script connecting per call hits it in normal use. Windows: handles grow. Remote API off by default. Fix: deregister closer when thread ends; seat after auth; back off on accept error.
2. Crash handler takes locks and allocates despite doc (crash.rs:30-33) and commit (reasoned). Crash while another thread loads a library (ORT providers, Vulkan drivers, FFmpeg hw libs: #326's window) can hang with nothing logged. Windows heap fault on non-Rust thread can refault in filter -> line lost. Crash during thread exit -> fail-fast attributed to concat.exe. Fix: write raw code + PC first; record module ranges at install; drop thread::current() from filter.
3. Android: every export, merge, card round opens a second Vulkan device (path verified, impact suspected). Android Vulkan (concat/src/gpu.rs:52-57) -> off_thread None (preview.rs:152); export/merge own instance+device; cards WgpuCompositor::new() every round (cards.rs:404-406). 3-4 GB phones, #312-class drivers. Fix: check Skia on Android (Vulkan or GL); if GL keep sibling().
LOW
4. api lib.rs:7 MCP.
5. Server::connections counts subscribers; stale "N connected".
6. API ConsolidateClips adopts any file (commands/clips.rs:612-645); AddMedia returns early if path in bin (commands/media.rs:19-25); no render-a-selection verb. Hide or add edit.consolidate.
7. Weak tokens accepted (settings.rs:575), no auth throttle (json.rs:267-291), 0.0.0.0 binds allowed (tests settings.rs:983, 996); LAN brute force -> writes anywhere under home, e.g. overwrite a file with PNG via preview.frame (api lib.rs:642-656).
8. Two crash codes can never arrive: STATUS_STACK_BUFFER_OVERRUN, STATUS_HEAP_CORRUPTION (crash.rs:358-359): fail-fast skips the filter.
9. Merge may bake a clip whose cutout masks aren't analysed (suspected); API export awaits masks (api lib.rs:574-580).
10. One-channel output loses right channel (playback.rs:1172-1175): HFP Bluetooth.
11. Audio callback frees memory on RT thread (playback.rs:1107).
12. README advertises gRPC API and CLI no download ships: releases build only -p concat (build-app.yml:523) without grpc feature (concat/Cargo.toml:133-134), no concat-cli shipped; window serves JSON-RPC over TCP only.
13. Update/model downloads no size limit (models.rs:258-276). Suspected: projects.rs:194-195 single rename no retry under OneDrive/AV on Windows -> "could not save".
14. Merged renders never collected (studio/consolidate.rs:102-109); undo keeps file; Clear cache spares media/.

### Numbers
host 11 971 (+1 129) tests 99 (22) +10, unsafe 42 (+38: crash.rs 32, machine.rs 6), unwrap 286 (2) (+37 all tests), panic 3 (0)
server 1 393 (0) tests 12 (4) unsafe 0 unwrap 28 (0)
api 2 352 (0) tests 21 (2) unsafe 0 unwrap 23 (0)
cli 517 (+3) tests 3 (1) unsafe 0 unwrap 2 (2)
Biggest files: playback.rs 1 347, api lib.rs 1 481, media.rs 1 093, titles.rs 878, updates.rs 862, cards.rs 653, templates.rs 618, crash.rs 556, json.rs 508.
Biggest fns: brush read 154, playback stream_for 147, cutout analyse 140, reverse footage 119, Server::start 95, api export 94.
Non-test unwraps: playback.rs:683, 693 (poisoned lock aborts); cli main.rs:208, 480.
CI: 8f5b141 green on three runners; main red since bdcaedf on i18n test.

### Scores (CQ Maint Scale Perf Arch Phil Sec Test)
host 7 6 5 6 6 6 4 7↑
server 7 8 5 6 8 8 4↓ 6 (rescored: pre-auth fd leak)
api 8 7 5 5 8 4 5 7
cli 7 8 – – 7 6 6 2
Features: Logs 7→8↑; ★Native crash reports 6; Self-update 5→5; Remote API 5→4↓; Monitor stills 7→7; ★Saving/autosave 6; Playback/audio 7→7; ★Merge (host half) 7; Windows data folders 6→6; ★Hardware info 7.

### Good
Crash line designed properly (stderr + dup'd fd, stack formatting, chains, before --version, real child-process test on 3 runners). Merge recheck before adoption + end-to-end test. Token compare constant-time shared by both transports, OS randomness, required on loopback, line/auth caps, auth timeout (token.rs:29-37, json.rs:35-43). Update file names refused if they escape (updates.rs:399-403 tested); saves sync before rename (projects.rs:183-195).
