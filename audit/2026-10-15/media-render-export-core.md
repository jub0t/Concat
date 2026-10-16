## media, render, export, core: reviewer 3, HEAD 11e1bd3 (includes merged PR #361 / 4f755b4)
Labels: Verified = read at line; Reasoned = followed path; Suspected = says why.
Commits in part: render, core none; media 447570a; export fc7e6a9, 4f755b4; 14b5998 (host/window) reviewed for export device.

### Re-verified
- 12Oct#4 Windows FXC compile unbounded, repeated, Cancel deaf: UNCHANGED. gpu.rs:2486-2560 outside DEVICE_WAIT; cancel only top of frame (concat-export/src/lib.rs:1524), progress after (:1687-1690); Windows export gets off_thread()=sibling() (panes/export.rs:518, preview.rs:152); drawing_in empty shader map (gpu.rs:861, :1267-1291) -> every effect recompiled every export.
- 12Oct#7 GLES 30s wait ~2.15s: UNCHANGED gpu.rs:1311-1318; wgpu 30 (src/Cargo.toml:35); doc gpu.rs:183-191 wrong.
- 12Oct#9 HEVC/AV1 export opens NVENC first: WORSE. lib.rs:1485 hardware:true; encoders(true) leads hevc_nvenc/av1_nvenc (encode.rs:117-130). New: background proxy reaches it (N3).
- 12Oct#10 validation error = lost GPU: UNCHANGED gpu.rs:3401-3437; recovered() second device, WARP, "lost 3 times" (lib.rs:1360-1385).
- 12Oct#11 combine unscoped, refused passes skipped, lent counter never moves: UNCHANGED gpu.rs:3450-3518, :2552-2558, :3044-3059, :839-842, :1291.
- 12Oct#31 8-bit phone fallback clips HDR: UNCHANGED gpu.rs:174-181, log only :788-795.
- 12Oct#33 DEVICE_WAIT doc, two read_back docs: UNCHANGED gpu.rs:183-191, 3312-3321.
- 12Oct#5 export side failed preview layer dropped silently: UNCHANGED lib.rs:2092-2123.
- 12Oct§7 encoder retry: FIXED (447570a) except Android launch check never exports (scripts/ci/smoke-android.sh).
- 11Oct#8 uncaptured GPU errors: PARTLY (rest is #11).
- 11Oct#11 mux scratch names collide: UNCHANGED lib.rs:1107-1109.
- 11Oct#12 two decode paths disagree past clip end: UNCHANGED, cause located: decode.rs:1072-1079 ends clip at last pts + OUTPUT frame duration; paced path drops layer (lib.rs:1615-1618), pool holds last frame (pool.rs:999-1008). 24/25p on 30/60p timeline to media end loses last output frame to black ~20% (reasoned).
- 11Oct#13 pick_rate VFR: UNCHANGED probe.rs:174-178, 230-245; no VFR fixture.
- 11Oct#14 MAX_SIDE doc: UNCHANGED frame.rs:27-31 vs gpu.rs:849-851.
- 11Oct#21 CPU fallback docs: UNCHANGED concat-export/src/lib.rs:18-21; gpu.rs:676-678, 1295-1296, 1644-1648.
- 11Oct#22 dead code core: UNCHANGED timeline.rs:314-321, 497-514.
- core Neg/ceil on i64::MIN: UNCHANGED time.rs:159-161, 251-259.
Tally 15: 0 fixed, 1 partly, 13 unchanged, 1 worse.

### Commits
- 447570a MediaCodec "try again" (#286): does it. send retries via patiently (encode.rs:1300-1327, 1408-1428) 10s patience 2ms pause; flush reads to EOF (:1330-1345, 1441-1455); software encoders unaffected. Two pure tests with fakes (:1640-1760). Misses: never run on phone, no CI export on Android; decoder mirror bug (N9) latent because Android never decodes on MediaCodec (N1); encoder-open reason thrown away (N7).
- 14b5998 own Vulkan device for export (#324): off_thread (preview.rs:142-153) sibling except Vulkan; reasoning right. Also used by consolidation. No test; rule inline. No effect on Android (no window device, concat/src/gpu.rs:15). Untested on Linux NVIDIA, the second-device road #202 found deadly on Windows NVIDIA laptops.
- fc7e6a9 consolidation: export part sound: effect_time_offset validated (lib.rs:1011-1015), quantise_at keeps grid (resolve.rs:226, 450-455), EffectClock shifts shader clock only (lib.rs:1734-1770); 4 host tests (concat-host/tests/export.rs:2005-2290) + 2 unit. Misses: EffectClock doesn't forward lost()/deliver_hdr() (N8); HDR consolidation on Android fails (HEVC 10-bit request concat-host/src/consolidate.rs:118-127, MediaCodec filtered from 10-bit encode.rs:551-561); one-line message.
- 4f755b4 centre transitions: correct, tested (lib.rs:790-830, extend :868-884, 6 unit tests replace 5, host wipe test updated). Misses: chained-lane bug kept (N2); sound crossfades in file only (N5); tail from media_duration = container length (longest stream) - 1 frame (:851-862), a phone file whose sound outlasts picture runs past last picture (suspected, Low); doc lib.rs:531-535 not rewrapped.

### New findings
HIGH
- N1 Android never decodes on MediaCodec; every phone clip CPU-decoded (reasoned from FFmpeg decoder tables). Device::attach uses decoder::find(context.id()) (hardware.rs:305-306) = built-in software decoder; open_codec opens same (decode.rs:741-743). FFmpeg's built-in h264/hevc hw configs never list MediaCodec (only separate *_mediacodec decoders) -> hardware_format None -> silent software decode. Yet hardware_wins says "MediaCodec takes everything" (hardware.rs:200-202), log "video decode on MediaCodec" (:358). Effect: 4K HEVC / 10-bit HDR phone footage CPU-decoded for playback/scrub/filmstrip/export: stutter, heat, battery; up to 8 warm readers (pool.rs:568-593) with frame threads on every core (decode.rs:731-735) on a 4 GB phone. No test (only macOS device test hardware.rs:534-544). Fix: open h264_mediacodec/hevc_mediacodec by name, buffer mode; fix N9 first.
- N2 Chain of transitions climbs lanes into next track's lane (verified by reading). overlap reads outgoing lane at resolve (lib.rs:806), incoming one lane above (:816). A->B->C on track t (lane 2t): B 2t+1, C 2t+2 = track t+1's lane for C's whole length; 4th clip 2t+3. Lanes used as-is (resolve.rs:217); clip_on_track_at one clip per lane (timeline.rs:472-480) -> wherever C overlaps a title/caption/overlay on track t+1, one isn't drawn. Monitor same (lib.rs:2194-2195). Slideshow with cross-fades + auto-captions. Predates 4f755b4; no test of two transitions in a row.
- N3 Importing >1080p HDR clip opens NVENC in background (worsens #9). Auto proxy for >HD (concat/src/studio.rs:3005-3032); HLG/PQ proxy HEVC hardware:hdr (concat-host/src/proxy.rs:224-243) -> hevc_nvenc first (encode.rs:117-130). On #309 hybrid-GPU laptops adding a phone 4K HDR clip segfaults in libcuda with unsaved edits. On Android proxy fails (10-bit filter encode.rs:551-561) so no proxy. Only escape CONCAT_HARDWARE_ENCODE=0, unmentioned.
MEDIUM
- N4 Audio mix holds every later clip's decoded sound; Cancel ignored. mix_to_file opens all pieces at once (audio.rs:405-470), round-robin (:638-685); adelay releases nothing until D -> ~whole timeline in filter queues, ~1.4 GB/hour 48k stereo float; OOM on phone. Speed-curved clip up to 400 inputs (lib.rs:1234). cancelled() only before/after (lib.rs:1130-1136), bar parked at 0 "mixing audio". (reasoned)
- N5 Transitions crossfade sound in file, not in window. Playback audio from session.flattened_clips() (concat/src/studio.rs:2663, 10544-10560; concat-host/src/session.rs:301-307) never through resolve_transitions (private lib.rs:550). Export fades both sides (lib.rs:812-822) and now plays outgoing past out-point. Editor hard cut; file crossfade + unheard source audio. (verified)
- N6 Android export sheet opens up to three MediaCodec encoders on UI thread. panes/export.rs:631-633 encodable() for H.264/HEVC/AV1 at first publish; each opens a 720p MediaCodec encoder (encode.rs:170-180, 472-500). Slower cold start; vendor codec hang -> ANR. (path verified, cost suspected)
- N7 FFmpeg's own error lines discarded. Log relay keeps only FATAL/PANIC as errors; ERROR/WARNING -> debug, dropped (ffi.rs:44-56, 101-105). Encoder refusal logs only "{name} would not open ...: Invalid argument" (encode.rs:1163). On #286 the explanation is thrown away.
LOW
- N8 EffectClock doesn't forward lost/deliver_hdr (lib.rs:1734-1770; compositor.rs:51-59 defaults). Latent.
- N9 Decoders drop a packet on EAGAIN (decode.rs:895-897, audio.rs:372-374, samples.rs:276-278). Harmless now; corrupts until keyframe once N1 fixed.
- N10 ~45 MB (8-bit) / ~80 MB (HDR) allocated per 4K exported frame: read_back (gpu.rs:3352-3380), write_frame (encode.rs:1489-1508) copies twice.
- N11 Prefetch doc wrong ("single-threaded" prefetch.rs:240-246); every reader frame-threads on all cores (decode.rs:723-735); 4 workers x all cores, 8 warm readers.
- N12 Android SDR proxy holds a MediaCodec encoder (proxy.rs:223-240); phones allow 1-2 sessions -> export mid-proxy may fail (suspected).
- N13 Volume doc drift: Animation::volume says factor (animate.rs:243-247; volume_at :280-283 no caller); mixer/playback absolute (audio.rs:61-70; concat-host/src/playback.rs:256-264).

### Numbers
media 10 328 (+262) tests 102 (+2) unsafe 38 unwrap 210 (23) panic 0
render 7 723 (0) tests 91 unsafe 1 unwrap 86 (9) panic 0
export 4 240 (+201) tests 35 (+1) + 15 host export tests (+4) unsafe 0 unwrap 31 (0) panic 0
core 2 897 (0) tests 49 unsafe 0 unwrap 26 (4) panic 2 (arena.rs:131, 187 unreachable) + 6 asserts time.rs
Biggest files: gpu.rs 3 561, export lib.rs 3 404 (+194), encode.rs 2 436 (+263), gpu/tests.rs 1 856, decode.rs 1 757, pool.rs 1 676.
Biggest fns: drawing_in 433 (gpu.rs:861), mix_to_file 303 (audio.rs:392), render_picture 268 (lib.rs:1428), open_video 238 (encode.rs:588), resolve_transitions_at 195 (lib.rs:554), Encoder::open 179.
All 38 media unsafe carry SAFETY; FFmpeg struct mirrors match 8.1. Shipped app panic=abort ([profile.app]) so any export-thread expect kills window; none looked reachable from user data. UI-thread blocking: encodable() (N6).

### Scores (CQ Maint Scale Perf Arch Phil Sec Test)
core 8 8 6 7 8 8 7 7: no commits.
media 7 6 4↓ 6↓ 7↓ 7 7 7: 447570a right; N1, N4, N7.
render 7 5 6 7 8 7 7 7: no commits; #4,#7,#10,#11 stand.
export 6↓ 4 5↓ 5 7 7 7 7: two contributor features well tested; lib.rs 3 404; N2; wrapper; one audio input per piece.
Features: Export desktop 8→7↓; Export on phone 6→6; Hardware encode 6→5↓; Hardware decode 7→5↓; Colour/HDR 7→6↓; Transitions 7→6↓; Crop/transform/blend/masks 7→7; Keyframes 7→7; Proxies 5→4↓; Playback decode 7→6↓.

### Good
447570a right fix, pure helpers tested with fakes, patience from last packet, stall = stated error. Contributor PRs with real tests (fc7e6a9 end-to-end phase+grid through reopened project; 4f755b4 tests for new rule incl. keys at instants). 14b5998 diagnoses cause, records per-API threading rule with issue numbers. No non-test unwraps in export; FrameRate::checked; sizes checked against device limits. render/core held still; GPU parity suite intact.
