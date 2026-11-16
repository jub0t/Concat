# Extras: final engineering audit (2026-11-15)

This audit covered the Extras system after its first two stages. It reviewed
every new and changed source file, ran the resource lifecycle in the
running app, reviewed the security model, and built and smoke-tested the
shipping profile. No features were added and nothing was re-architected.
Each fix below is small and has a test. Each test was seen to fail without
its fix where the failure is deterministic.

See [EXTRAS_TESTING.md](EXTRAS_TESTING.md) for every test and live run, and
[EXTRAS_SECURITY.md](EXTRAS_SECURITY.md) for the security model with the
residual risks this audit added.

## Findings

### Medium (fixed)

**M1. The install database had no writer lock.** An install commits on a
worker thread. Switching or removing an extra runs on the window's thread.
Several Concat windows (separate processes, which Concat allows) can share
one data folder. Each operation read `installed.json`, changed it and wrote
it back with nothing in between to stop another writer:

- a switch made during an install's commit could be undone
- an extra's record could be lost
- two saves racing on `installed.json.new` failed an install outright

An install also checked its dependencies only before unpacking. A
dependency removed in the meantime let it in without one.

- **Fix**: an exclusive OS file lock (`extras/extras.lock`) around every
  registry read-modify-write and every move into or out of `packages/`.
  The lock is never held across a download or an unpack. An install checks
  its dependencies (and its no-downgrade rule) again under the lock.
  (`concat-extras/src/install.rs`)
- **Tests**:
  - `installs_and_switches_on_two_threads_lose_nothing` fails 3 runs out of
    3 with the lock disabled, and passes with it.
  - `a_dependency_removed_during_an_install_stops_it`.

**M2. A second window could sweep away the first one's install.** At start,
`sweep()` deletes `staging/` and `trash/` and "recovers" updates it thinks
were interrupted. A window started while another was mid-install would
delete that install's staging folder, or move its packages.

- **Fix**: each running window holds a shared lock on `extras/running.lock`.
  A starting window sweeps only if it can take that lock exclusively.
  (`install::sweep`, `install::attend`, `concat_host::extras::sweep`)
- **Tests**: `a_sweep_leaves_alone_what_a_running_concat_is_doing`.
- **Live**: a second `concat.exe` logged "another Concat is using …; what it
  has in progress is left alone" and kept a planted staging folder. The next
  start with the folder to itself swept it.

### Low (fixed)

| # | Finding | Fix | Evidence |
|---|---|---|---|
| L1 | `concat-cli extras keygen` checked for an existing file, then wrote it: a race could overwrite a key. On Unix the secret key was world-readable (default mode) | `create_new` (one step), mode 0600 on Unix | `a_secret_key_is_never_written_over_one_already_there` |
| L2 | After a sticker pack update, the Pictures card kept drawing the old picture: the art cache was keyed by id, which an update keeps | Cache cleared on every refresh | Live: the updated Gold Star card turned green |
| L3 | The From Extras sound list is read on a worker. Two refreshes close together could land out of order and show a stale list | Only the latest request's result is used (a generation counter) | Reviewed. A race, not deterministically testable |
| L4 | Placing an updated sound or sticker reused the project's copy when it had the same name **and size**. An updated WAV of the same length is the same size with different samples | Same size and same bytes before reusing. Otherwise the copy takes a numbered name. Nothing is overwritten | Unit test. Live: `Rising Whoosh 2.wav` (new hash) beside the original |
| L5 | The removal warning listed the project's effects from an extra, not the fonts its titles use from it | Fonts from the extra's folder are named too. The message no longer says "effect" | Live: "The open project uses Monoton from it." The new wording passed the locale check; it is in the shipping build but was not seen on screen |
| L6 | Hex parsing accepted a sign (`+a` read as `0a`), so a key or signature had two spellings | Hex digits only | `hex_round_trips_and_refuses_what_is_not_hex` |
| L7 | An `installed.json` edited by hand or damaged could name an id or root such as `../..`, which became a path outside the folder (read, offered, or deleted on removal) | Such records are ignored when read | `a_record_that_leads_out_of_the_folder_is_ignored` |
| L8 | `CONIN$`, `CONOUT$`, `COM¹`–`COM³`/`LPT¹`–`LPT³` and `NUL .txt` were not treated as Windows device names | Added | `safe_path` test cases |

### Low (not fixed: documented)

- **L9. Updating a switched-off extra switches it on.** `Record::new` always
  records an update as enabled. The resolver relies on this when a
  switched-off dependency needs an upgrade. Keeping the person's choice
  would mean changing the resolver, the queue (which keys steps by id) and
  the install API. That is more than a stabilisation pass should change.
  Seen live: Test Sounds, off, came back on after Update all.
- **L10. The archive is hashed, then reopened to unpack.** Only another
  program running as the same user, with write access to the downloads
  folder, could swap it in between. See EXTRAS_SECURITY.md.

### Informational

- **Error kinds are recognised by text.** The sheet treats an error
  containing "cancelled" as a cancel. The host deletes a download whose
  error mentions "SHA-256". An extra whose name contains "cancelled" would
  have a real failure logged as a cancel.
- **Each catalogue reload leaks the old catalogue** (`Box::leak`, so it can
  be `&'static`). This pattern predates Extras. Extras add reloads on every
  install, removal and switch, so a long session leaks a little more.
- **A project's font entries are absolute paths.** `ensure_font` records
  the file behind a picked family, which for an extra's font is inside
  `extras/packages/`. After removal that entry dangles and titles fall
  back, unless another entry carries the family. A preset's font is copied
  into the app's `fonts/` folder, which is why the test project's title
  kept Monoton after the extra was removed. App fonts already worked this
  way before Extras.
- Removed dead code: GIF and SVG checks in the image sniffing, which no
  extra kind accepts any more.
- Corrected the docs: `concat-cli serve` loads packages once at start (its
  `Api` factory is `FnOnce`), not per connection.
- Checked and found sound:
  - Shader pipelines are cached under a fingerprint of the shader source,
    so an updated effect draws with its new shader at once (measured live).
  - The two Slint deprecation warnings in the build are in code this work
    did not change.

## Repository and environment integrity

Results are in the final report and in [EXTRAS_TESTING.md](EXTRAS_TESTING.md#final-audit-2026-11-15).
