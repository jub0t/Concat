# Extras: tests

Every check in the security model and the install pipeline has a test. This
page lists them and what was actually run on Windows 11 x64 (Rust 1.93.1,
MSVC 14.52) when the system was built. See
[DEVELOPMENT_ENVIRONMENT.md](DEVELOPMENT_ENVIRONMENT.md) for the environment.

## Running them

```sh
cd src
cargo test -p concat-extras                    # the engine: format, index, plans, archive, registry, install
cargo test -p concat-host --lib extras         # repositories, fetch, cache, download, vet, end to end
cargo test -p concat-effects                   # catalogue over several roots
cargo test -p concat-api                       # extras.list
cargo test -p concat-api --features schema --lib contract   # the API's published schema
cargo test -p concat --lib panes::extras       # the sheet's pure helpers
cargo clippy -p concat-extras -p concat-host -p concat-api -p concat-cli -p concat-mcp --all-targets -- -D warnings
cargo clippy -p concat --lib --profile test -- -D warnings
cargo fmt --all -- --check
python scripts/locales.py --check
```

## What is covered

### Package format and manifests (`concat-extras`, `manifest.rs`, 10 tests)
- A valid manifest round-trips. Unknown fields and a newer format are refused.
- Id rules. Path safety: absolute paths, `..`, `.`, `//`, a trailing `/`,
  `C:`, `\`, NTFS streams, `CON`/`COM1`/`LPT9`, trailing dots and spaces,
  control characters, over-long paths.
- Files must sit under a root of a kind that takes them, and `.exe` is
  refused. Duplicates are refused, including duplicates by case. A missing
  digest is refused. A path that is both a file and a folder is refused.
- Overlapping roots, self-dependencies and bad version ranges are refused.
  Text field limits, https-only homepages, `min_app` and platform checks.

### Archives (`archive.rs`, 6 tests)
- A packed extra unpacks to exactly what it lists. Packing is deterministic.
- **Hostile archives**: traversal (`../`, absolute, `\`, `C:`), symlinks,
  unlisted files, missing files, altered bytes, wrong sizes, duplicate
  members, empty folders, a manifest that isn't first, a manifest that
  doesn't parse. Each is refused and leaves nothing on disk.
- A non-gzip file is refused.
- **Decompression bomb**: an 8 MB member behind a manifest that lists 4 bytes,
  compressed to under 64 KB, is stopped.
- A cancel stops unpacking and hashing, and cleans up. An existing staging
  folder is never written into.

### Signed index (`index.rs`, 7 tests)
- A signed index verifies. An altered byte, another key, no key, or no
  signature are all refused.
- **Expired**: an index past its signed `expires` is refused, and one with
  `expires = 0` never expires.
- **Rolled back**: an index whose `serial` is lower than one already seen is
  refused. The same serial is accepted.
- **Malformed and misdirected signatures**: a signature that is too short or
  too long, has non-hex characters or whitespace inside it, was made over
  other bytes, or was made by another key is refused. A key of the wrong
  length is refused. Whitespace around the signature is trimmed. A correctly
  signed file that isn't an index is refused as invalid.
- Entry checks: duplicates, `http://` downloads, `../` URLs, bad versions,
  a zero size, a newer schema, invalid JSON.
- Relative URL resolution for https and `file:`. Search, and which extra
  provides an id.

### Dependencies (`resolve.rs`, 8 tests)
- No dependencies. Chains come out dependencies-first, each once.
- An installed dependency that satisfies the range is left alone. A
  switched-off one is enabled.
- A missing dependency and a version nothing satisfies are reported, not
  guessed.
- **Cycles are refused**, with the cycle named.
- Upgrade, reinstall, and **never a downgrade**.
- An upgrade that would break an installed dependent is a conflict.
- Dependents (for remove and switch-off).

### Registry (`registry.rs`, 6 tests)
- Round-trip. Only enabled extras' roots are offered. A root missing on disk
  is not offered.
- A broken or newer registry reads as empty and deletes nothing.
- The stamp changes when the registry is written. Provided ids merge.

### Install pipeline (`install.rs`, 15 tests)
- An install lands in place, goes through the phases in order, records last
  and leaves no staging folder.
- A wrong digest, size, id or version installs nothing.
- A refused vet installs nothing.
- Upgrade, repair by reinstalling the same version, **refused downgrade**,
  and an explicit downgrade.
- Dependencies must be installed and switched on. A needed extra cannot be
  removed or switched off. Remove and switch-off work in order.
- Incompatible `min_app`. A cancel leaves nothing. Sweep clears staging and
  trash and keeps partial downloads.
- **Interrupted updates**: an update killed after the old version went to
  `trash/` but before the new one landed, and one killed after the new one
  landed but before the registry was written, are both put back to the
  recorded version by the next start's `recover()`.
- **A failed registry write** during an update (the temporary file's path is
  made a folder) rolls the update back: the old version is in place and the
  registry still names it.
- **Dependency conflict at install**: an update that an enabled dependent's
  range doesn't accept is refused, even when called without a plan.

### Authoring (`pack.rs`, 3 tests; `lib.rs`, 3 tests)
- Keys sign what their public half verifies.
- Packing lists every file, skips hidden ones, finds effect ids, and refuses
  unsupported types.
- A built repository verifies end to end.

### Host (`concat-host`, `extras.rs`, 20 tests)
- **The documented example** (`docs/development/extras-example`) packs,
  signs, installs, and both its shaders load.
- A local `file:` repository installs end to end. Its effects land in the
  roots the catalogue loads, and the download is not kept.
- An effect whose shader doesn't compile is refused at install. A package
  under `concat.` is refused.
- **A tampered archive** is refused by digest. **A tampered index** is refused
  by signature.
- **A download longer than the index says** is stopped while it is read.
- **A cancelled download** installs nothing.
- An unreachable repository falls back to its verified cache, marked stale.
- A chosen file needs a vouching index or an explicit yes, and is recorded
  as verified or unverified accordingly.
- Fonts must parse. Images must match their magic bytes.
- Insufficient free space is refused. A repositories file with no key or
  plain http is reported and not trusted. `file:` URL decoding.
- **A replayed older index** (lower serial, correctly signed) is refused, and
  the newer cached one stays in use.
- **An expired index** is refused from the repository.
- **A changed key** never reads the old key's cache: the cache folder is
  keyed by address and key.
- **The official name is reserved**: a configured repository called "Concat
  Extras" (any case) is reported and not used.
- An extra's sticker and sound files are listed by kind and extension, and
  vanish when it is switched off. `effect_roots` puts the user's folder first.

### API (`concat-api`, 1 new test, plus the schema)
- `extras.list` reports the install database (id, version, enabled,
  verified, kinds, provided ids). The published schema includes it.

### Effects catalogue (`concat-effects`, 1 new test)
- Packages load from several roots. A repeated id is refused from the later
  root, and the user's own wins. A vet refusal is reported. The stamp covers
  every root.

### Window (`concat`, `panes/extras.rs`, 3 tests; `panes/sounds.rs`, 1 test)
- Shelves map to categories. Progress reports at most once per percent and
  per phase. Long lists are shortened to their first three names.

## Results (as run)

Final run, second stage (2026-11-15):

| Command | Result |
|---|---|
| `cargo test -p concat-extras` | **54 passed**, 0 failed (47 in the first stage) |
| `cargo test -p concat-host --lib` | **186 passed**, 0 failed, 4 ignored (167 before Extras, 180 in the first stage) |
| `cargo test -p concat-effects` | **75 passed**, 0 failed (74 before Extras) |
| `cargo test -p concat-api` | **31 passed**, 0 failed |
| `cargo test -p concat-api --features schema --lib contract` | **2 passed** |
| `cargo test -p concat-cli` | 3 passed |
| `cargo test -p concat --lib panes::extras` | **3 passed**, 0 failed |
| clippy, `-p concat-extras -p concat-host -p concat-api -p concat-cli -p concat-mcp --all-targets -D warnings` | clean |
| clippy, `-p concat --profile test -D warnings` | clean |
| `cargo fmt --all -- --check` | clean |
| `scripts/locales.py --check` | passes (exit 0). 97 new keys in `en.json`, which the other locales list as missing and show in English until translated. 0 stale |
| `concat-cli check` on the example's effects | 2 packages ok |
| `concat-cli extras` smoke test (keygen, repo, verify, inspect, plus a tampered index, a tampered archive, and keygen over an existing key) | all behaved as intended. Both tampered cases were refused. The existing key was not overwritten |

Pre-existing failures, unrelated to Extras and present before any change on
this machine: three `concat-host` export integration tests
(`one_clip_exports_whole_at_every_rate`, `the_edges_export_too`,
`every_edit_still_exports`) find the exported audio a few tens of
milliseconds short.

## In the running app

`concat.exe` (debug build) ran in portable mode, so all data stayed beside
the executable. A local repository was signed with `concat-cli extras repo`
and listed in `extras/repositories.toml`. The window was driven through
Windows UI Automation and captured window-only:

1. The **Extras** button appears beside Settings in the title bar on the
   launch screen. It has an accessible name and default action, and opens
   the sheet.
2. **Discover** lists the Starter Pack from the signed index. The footer
   names the repository and revision, and says the official repository isn't
   set up in this build.
3. **Install** downloads, verifies, unpacks, vets and commits. A toast says
   "Starter Pack is installed." `installed.json` records it with its digest
   and provided ids. No staging is left behind.
4. The effects catalogue reloaded with both packages. Their shaders passed
   the GPU trial, and Concat drew their effect cards: **Warm Glow** tints
   the card amber, and **Diagonal Wipe** shows the cut mid-way.
5. The extra's page shows its description, licence, versions and sizes, what
   it brings, and its signed source.
6. **Switch off** sets `enabled: false`, the badge reads "Switched off", and
   the cards are pruned, because the catalogue dropped the packages.
7. **Remove** asks first through the shared confirmation sheet, then deletes
   the files and the record, leaves no trash, and shows "Starter Pack was
   removed."
8. **AI models** shows the existing transcriber and voice rows with their
   sizes and Download buttons. No model was downloaded during the test.

### Second stage: the whole workflow in the window

A rebuilt debug `concat.exe` ran in portable mode against a local repository
of five test extras, signed by `concat-cli extras repo --valid-days 30`:

- Starter Pack: an effect and a transition
- Monoton Font: a real OFL font from google/fonts
- Neon Titles: a title preset that requires Monoton `^1`
- Test Stickers: two PNGs
- Test Sounds: two WAVs

Buttons were pressed through Windows UI Automation's Invoke pattern. That
only works because the shared `Button` now exposes a role and a default
action. Screenshots are in `NeededDownloadsAndInstall/temp/gui/`.

| Step | What happened |
|---|---|
| Discover | Lists all five from the signed index, revision 2 |
| Install | Neon Titles queued Monoton Font first and installed both. Stickers, Sounds and Starter Pack installed too |
| Use: effect | Warm Glow applied to a clip, with the fx badge. In the API's `preview.frame`, mean RGB over the same crop went from (128,130,127) without it to (129,130,120) with it, which is warmer |
| Use: transition | Diagonal Wipe between two clips draws the diagonal cut in the window. The monitor's mean colour was (127,131,119) with the transition lane shown and (125,131,126) with it hidden. The headless preview draws no transition, built-in or extra, which is existing behaviour |
| Use: font | The font picker offers Monoton. The "EXTRAS" title renders in it, in the window and through the API's `preview.frame` |
| Use: title preset | Text & Captions → Styles shows Neon. Applying it set Monoton, colour `#ff4fd8` and the text "Neon" |
| Use: sticker | Stickers → Pictures (2) shows both pictures. Pressing Gold Star copied it to the project's `stickers/` and placed it over the picture at the playhead |
| Use: sound | Audio → From Extras (2) lists both sounds with their lengths. Rising Whoosh was copied to the project's `audio/` and placed |
| Save, restart, reopen | The project reopened with 3 media, 5 tracks and 7 clips, plus the fx badge, the Monoton title, the transition and the sticker |
| Switch off | Starter Pack switched off. The effect stopped drawing at once. On reopening, a notice named it: "This project uses effects from Starter Pack, which is not installed or is switched off." The project kept both ids |
| Switch on again | The effect drew again at once. **A bug was found and fixed here**: the monitor kept a cached plan without the effect until the next edit. `reload_packages` now drops it. A colour patch read 242 (on), 255 (off), 242 (on again) |
| Remove, refused | Removing Monoton Font was refused while Neon Titles (enabled) needs it |
| Remove, warned | Removing Starter Pack warned that the open project uses `example.diagonal-wipe` and `example.warm-glow` from it, then removed it after confirmation. No staging or trash was left |
| Remove stickers and sounds | The Pictures and From Extras shelves emptied at once and showed their empty states. The placed Gold Star still drew from the project's copy |
| After closing | `concat.json` still links both effect ids. `stickers/Gold Star.png` and `audio/Rising Whoosh.wav` are still in the project. The log ends "run ended cleanly" |

The headless side was checked against the same installed extras through
`concat-cli api`:

- `extras.list` returned all five.
- `catalogue.list` included both Starter Pack effects.
- A project built through the API, with the effect, the transition, the
  Monoton font and a title, saved and rendered.

Not exercised in the running app:

- an HTTPS repository (there is none to point at yet)
- downloads over 100 MB (the confirmation)
- installing an unverified file from the file picker
- a real screen reader (UI Automation was used instead)
- dragging a picture sticker, which is not wired

The unit and host tests above cover these paths where tests exist. None of
them had a live run.

## Final audit (2026-11-15)

The audit is written up in [EXTRAS_AUDIT.md](EXTRAS_AUDIT.md).

### New tests

| Test | What it shows |
|---|---|
| `install::installs_and_switches_on_two_threads_lose_nothing` | 12 installs on one thread and 40 switches on another lose no record and undo no switch. With the folder lock disabled it failed 3 runs out of 3 |
| `install::a_dependency_removed_during_an_install_stops_it` | A dependency removed while an install unpacks (done from its vet) stops the install, and nothing is left behind |
| `install::a_sweep_leaves_alone_what_a_running_concat_is_doing` | Staging and trash are kept while another Concat holds the folder, and swept once none does |
| `registry::a_record_that_leads_out_of_the_folder_is_ignored` | A record whose id or root reads `../..` is ignored |
| `lib::hex_round_trips_and_refuses_what_is_not_hex` (extended) | `+a` and `-0` are refused |
| `manifest` path cases (extended) | `CONIN$`, `CONOUT$`, `COM¹`, `NUL .txt` refused; `COM10`, `console` allowed |
| `concat-host` `an_updated_effect_answers_with_its_new_shader_and_a_removed_one_is_gone` | After an update, the same effect id loads the new shader and only the new version is on disk. After removal the id no longer loads |
| `concat-cli` `a_secret_key_is_never_written_over_one_already_there` | `keygen` never overwrites a key (and on Unix makes it owner-only) |
| `concat` `same_contents_tests` | A project copy is reused only when its bytes match, not just its size |

### Resource lifecycle, in the running app

These ran on the debug build in portable mode, on the test project. A
repository revision 3 (`gui-repo/out3`, same key, higher serial) offered
1.1.0 of three extras:

- Starter Pack 1.1.0: Warm Glow's shader lowers blue far more.
- Test Stickers 1.1.0: Gold Star recoloured green, same file name.
- Test Sounds 1.1.0: Rising Whoosh with other samples, exactly the same
  size (144,044 bytes).

| Case | Result |
|---|---|
| Second window while the first runs | It logged "another Concat is using …; what it has in progress is left alone" and kept a planted staging folder. The first window was unaffected |
| Install 1.0.0 of all three from revision 2 | Installed. The project's existing effect links drew again. Monitor cyan bar (0,255,242) |
| Disable sounds placed on the timeline | Test Sounds switched off. The project's copy stayed on the timeline |
| Remove a font that titles use | Neon Titles switched off first (it needs Monoton). The removal warning named the font: "The open project uses Monoton from it." After removal the title still drew in Monoton, because the project also carries the copy of the font the Neon preset put in the app's `fonts/` folder. The project kept both font entries |
| Update effects used by the open project | Updates (3) appeared after Refresh. Update all: the monitor's cyan bar went from 242 to **174** with no restart, so the new shader drew. The project's links were unchanged |
| Update a switched-off extra | Test Sounds came back **switched on**. This is the documented Low finding L9 |
| Update a sticker pack | The Pictures card redrew as the green star. Placing it made `Gold Star 2.png` (1,014 bytes) beside the untouched `Gold Star.png` (1,026 bytes) |
| Update a sound to one of the same size | Placing it made `Rising Whoosh 2.wav`, whose hash matches the new file (`13450A58…`). The original (`93B7D83A…`) was not reused or overwritten |
| Remove packages whose content is in use | Starter Pack's warning named both effect ids. Stickers and Sounds removed. The effect stopped drawing (cyan 255). The placed green star still drew from the project's copy. The From Extras shelf showed its empty state. No trash left |
| Reopen after the updates and removals | Graceful close ("run ended cleanly"), then restart. The window, now alone, swept the planted staging folder. The project reopened with 5 media, 7 tracks and 9 clips. The missing-extras notice named both ids. `concat.json` still links `example.warm-glow` and `example.diagonal-wipe`, and all four copied files are in the project |

### Shipping build

`cargo build --profile app -p concat` (fat LTO, `panic = "abort"`) took
34 min 24 s, and `concat.exe` is 105.5 MB. It was staged as CI's "Stage
(Windows)" step does: the program, FFmpeg's DLLs, ONNX Runtime's DLLs,
`LICENSE`, `THIRD_PARTY_NOTICES.md`, and the Visual C++ runtime DLLs that
something in the folder imports. The checks of `scripts/ci/smoke-windows.ps1`
then ran, ported to Windows PowerShell 5.1 (the script needs 7, which this
machine doesn't have):

1. Every DLL anything in the folder imports is in the folder or is
   Windows' own: **pass**.
2. `concat.exe --version` with PATH cut down to Windows' folders: **exit 0**.
3. `concat.exe --smoke` the same way: **exit 0**. The window came up on the
   RTX 3050 (Direct3D 12) and the run ended cleanly.

Then the staged program ran with its own `portable/` folder and PATH set to
Windows only. In it:

- the Extras sheet listed all five extras from the signed local repository
- installing Neon Titles installed Monoton Font first
- Starter Pack 1.1.0 installed, and its shaders were vetted
- the log has no warnings or errors, and the run ended cleanly

The program has no runtime dependency on the development machine's paths.
Its strings do contain this machine's build paths: crate sources for panic
messages, C `__FILE__` in whisper.cpp, and generated `OUT_DIR` files. That
is normal for Rust and C builds, and CI's builds carry CI's paths the same
way. It is not a dependency.

Differences from CI's build:

- whisper's ggml was built with `GGML_NATIVE=OFF` and linked statically.
  CI builds the CPU kernels as one DLL per CPU generation
  (`GGML_BACKEND_DL`). That needs the Windows SDK's `rc.exe` to embed DLL
  manifests, and this environment has only `llvm-rc`, which failed on the
  linker's manifest script. The CPU-variant DLLs are therefore not covered.
- Nothing was signed with Authenticode. The emulated Ivy Bridge check
  (Intel SDE) was not run.

### Final regression run (on the final source)

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo build -p concat -p concat-cli` (debug) | built, 9 min 36 s |
| `cargo build --profile app -p concat` (shipping) | built, 34 min 24 s |
| `cargo test -p concat-extras` | **58 passed** (54 before the audit) |
| `cargo test -p concat-host --lib` | **187 passed**, 4 ignored (186 before) |
| `cargo test -p concat-effects` | 75 passed |
| `cargo test -p concat-api` | 31 passed |
| `cargo test -p concat-api --features schema --lib contract` | 2 passed |
| `cargo test -p concat-cli` | **4 passed** (3 before) |
| `cargo test -p concat --lib -- panes::extras panes::sounds` | **4 passed** |
| `cargo clippy -p concat-extras -p concat-host -p concat-api -p concat-cli -p concat-mcp --all-targets -- -D warnings` | clean |
| `cargo clippy -p concat --profile test -- -D warnings` | clean. It first caught a test module placed mid-file (`items_after_test_module`), which was fixed. The only other output is Slint warnings in code this work did not touch |
| `scripts/locales.py --check` | passes (97 new keys untranslated, 0 stale) |
