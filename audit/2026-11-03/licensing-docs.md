## Licensing, notices and docs. main eda5e0af, 2026-11-03

Labels as in the [engine report](engine-export.md). This is a reading of the repository's own documents, not legal advice.

### 1. What changed

- `09f1a5d` and `00da96c`:
  - Concat is offered under the AGPL-3.0-or-later or a commercial licence (`LICENSING.md`).
  - CLA 3.1 assigns copyright to the owner, licenses it back to the contributor, and promises that what they contributed stays available under the AGPL.
  - `audit/CLEAN-ROOM.md` lists the outside code to rewrite.
- `b8b6528`, `32945bc` and `e63756d` remove every GPL component the builds carried: x264, x265, and espeak-ng through sherpa-onnx. Every FFmpeg Concat ships is now an LGPL build.
- `3c5ffe8`: the AGPL build takes Slint under GPL-3.0, and a commercial build would take it under Slint's royalty-free licence.

### 2. Findings

**D-M1. A commercial licence is offered for code the owner does not yet hold.** *Verified.*
- `CLEAN-ROOM.md` counts outside contributions still on `main` as of 2 November: **141 commits, 19 274 lines in 327 files**. 0 of the 141 items are ticked.
- The largest single item is `6f2e3ae7`, 3 068 lines across the effects engine, the lanes and `gpu.rs`.
- Until those lines are rewritten, or assigned under the CLA, a commercial licence can't cover them. `LICENSING.md` §2 does not say so.
- **Fix:** add a sentence to `LICENSING.md`: "until the clean-room list is done, a commercial licence covers …". Or hold commercial offers until the list is done.

**D-M2. The notices describe components that are gone, and leave out one that now ships.** *Verified.*
- `THIRD_PARTY_NOTICES.md:153-180` still says sherpa-onnx is "linked statically", that espeak-ng is "compiled into the app binary", and that Kokoro and Pocket TTS are downloaded. `e63756d` removed all of them.
- `LICENSING.md` still names "the GPL builds of FFmpeg with x264, espeak-ng" as components a closed build can't take.
- `bria.flac` now ships inside the app (`concat-speech/voices/`), but the notices don't mention it. The voice's only note is `voices/README.md`.

**D-M3. The bundled voice's licence is taken from a repository's code licence.** *Suspected.*
- `voices/README.md` gives `bria.flac` the licence of Kyutai's delayed-streams-modeling *code* (MIT and Apache-2.0).
- It is a recording of a person's voice. The notices used to put the bundle it came in (Pocket TTS) under CC-BY-4.0, and to repeat Kyutai's request that no one's voice be cloned without consent.
- **Fix:** find the recording's own licence and consent terms. If it is CC-BY, attribute it in the notices. If they are unclear, replace it with a recording Concat owns, for example the owner's own voice.

**D-L1. The README promises text to speech on every platform** (`README.md:49, 62`). Since `e63756d`, "a phone offers no voice" until Android's system TTS is wired in.

**D-L2. A commercial build with phone FFmpeg statically linked.** The phones link an LGPL FFmpeg statically. This is fine under the AGPL, because the full source is offered. A closed-source build must give its users a way to relink, and `LICENSING.md` leaves that to the commercial agreement. It's worth stating in that agreement's template.

**D-L3. Commit messages contradict each other.** See engine X-L1: `9e69bbc` says the fallback is x264.

### 3. What is right

- The codec-patent section in `LICENSING.md` is accurate, and the OpenH264 note says outright that BtbN's build carries no Cisco patent licence.
- SPDX headers drop "and Concat contributors" only once a file's last outside line is gone (`a465eb0`, `98f04de`). That is the rule CLEAN-ROOM.md sets.
- No GPL code is left in any build. This removes the main obstacle to a store listing or a commercial build.

### 4. Score

| Aspect | 24 Oct | Now | Why |
|---|---|---|---|
| Documentation | 5 | 4 ↓ | The licensing story is clearer, but the notices and the README run behind the code on TTS and x264, and one bundled asset is unattributed |
