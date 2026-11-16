# Extras: security model

Extras download content from the internet into the app. This page lists what
an attacker could try and what stops each attempt. Every check named here has
a test (see [EXTRAS_TESTING.md](EXTRAS_TESTING.md)).

## Chain of trust

```
repository key (compiled in, or in the person's repositories.toml)
  └─ Ed25519 signature over the exact bytes of index.json
       └─ the index's serial (never lower than one seen) and expiry
            └─ each entry: the archive's size and SHA-256
                 └─ extra.toml inside the archive: every file's path, size and SHA-256
```

### The signed index

- **Refused unless signed** by the repository's configured key. There is no
  unsigned mode. A repository with no valid key is reported and never asked.
- **Verification** (`concat_extras::index::verify`) uses ring's Ed25519 on the
  raw bytes. The public key must be exactly 32 bytes of hex and the signature
  exactly 64. A signature of a different length, with non-hex characters, with
  whitespace inside it, made over other bytes, or checked against another key
  is refused, and so is an index changed by one byte. Only hex digits are
  read: a sign such as `+`, which Rust's number parser would otherwise
  accept, is refused, so a key or signature has exactly one spelling.
  Whitespace around the signature file's contents is trimmed. An index that is signed but isn't a
  valid index is refused as invalid. Each case is tested.
- **Expiry**: an index may carry `expires` (Unix seconds, signed with
  everything else). After that moment it is refused, from the network and
  from the cache. That limits how long a stolen or withheld index can be
  served. `concat-cli extras repo` sets it 365 days out by default
  (`--valid-days`).
- **Anti-rollback**: an index carries a `serial` the repository raises on
  every publish. Concat remembers the highest serial it has verified for each
  repository (`cache/<repo>/serial`), and refuses an index with a lower one.
  Without that, an attacker could serve an old signed index that still lists
  an archive with a known problem. When this happens the verified newer copy
  stays in use, marked stale, with the reason in the sheet's footer.
- **Transport**: fetched over `Trust::Shipped` (Mozilla's roots only), like the
  update and sound manifests, so a root a proxy installed on the machine
  cannot re-sign it. The signature would catch that anyway.

### Key management

- **Where keys come from**: the compiled-in official key
  (`concat_host::extras::OFFICIAL_KEY`), and `repositories.toml` in the
  person's own config folder. **Nothing a repository serves can supply or
  change a key.** The index has no key field. A download cannot write to
  `repositories.toml`. A configured repository may not take the official
  repository's name (compared without regard to case).
- **Caches are keyed by address and key together**
  (`cache/<name>-<sha256(index URL, key)[..12]>`). If a repository's key is
  changed, by the person or in a new build, the index the old key signed is
  never read again. Two repositories whose names read alike never share a
  cache. Every cached index is verified again on every read, because it is a
  file anyone on the machine could edit.

### Archives

- Fetched over `Trust::System` through the existing resumable downloader.
  That is safe because their digest is already known from the signed index.
- **Cut off** as soon as more bytes arrive than the index states.
- Size and SHA-256 are checked before the archive is opened.
- The manifest inside must name the same id and version the index offered.
- A **file the person chooses** is verified when its SHA-256 matches an entry
  in a signed index. Otherwise the sheet says it is unverified and asks
  before installing, and the install database records it as unverified.

## Archive handling (`concat_extras::archive`)

| Attack | Defence |
|---|---|
| Path traversal (`../`, absolute paths, `C:`, `\`, NTFS streams `a:b`) | `safe_path`, on the manifest and again on every tar member |
| Windows device names (`CON`, `NUL`, `COM1`…, `CONIN$`, `CONOUT$`, `COM¹`…, and `NUL .txt`), trailing dots or spaces | refused by `safe_path` |
| A tampered `installed.json` naming an id or root outside the folder | that record is ignored, so nothing is read, offered or deleted there |
| Case collisions (`A.toml` and `a.toml`) | refused in the manifest |
| Symlinks, hard links, devices, FIFOs, sparse files | refused member types |
| Unlisted extra files | refused, since the manifest is the whole list |
| Missing, short, long or altered files | size from the tar header, then bytes counted while streaming, then SHA-256 per file |
| Duplicate members, or a second manifest | refused, and files are created with `create_new` |
| Decompression bombs | the decompressed stream is capped at what the manifest declares plus tar overhead |
| Huge manifests | 4 MiB limit, read with a cap |
| Oversized content | 4 GiB per file, 8 GiB in total, 20,000 files, plus a free-space check before downloading |
| Executable content | per-kind extension allowlist. No kind accepts `.exe`, `.dll`, `.so`, `.dylib`, scripts or archives. Stickers are limited to PNG, JPEG and WebP |

## Installs, updates and recovery (`concat_extras::install`)

- **Order of operations**:
  1. The archive is verified, then unpacked into `staging/` with every check.
  2. The kind's vet runs.
  3. The installed version, if any, is renamed into `trash/`.
  4. The new version is renamed into `packages/<id>/`.
  5. `installed.json` is written last, through a temporary file and a rename.
- **A failure** at step 4 or 5 puts the previous version back where it was and
  leaves the registry as it was. This is tested by making the registry
  impossible to write mid-update.
- **A crash** at any point is repaired on the next start by `recover()` (run
  inside `sweep()`). The registry is the record of what finished. If
  `packages/<id>` is missing, or holds a version the registry doesn't name,
  the recorded version is moved back from `trash/` and the unfinished one is
  cleared away. Both crash points are tested. Only then are `staging/` and
  `trash/` emptied.
- **Dependencies**: an install is refused if it needs an extra that isn't
  installed and switched on. **An update is refused if it would break an
  enabled extra that depends on it.** This is checked at install, not only
  when planning. Removing or switching off an extra that others need is
  refused. Cycles are refused when planning.
- **Downgrades** are refused. Reinstalling the same version repairs a damaged
  install.
- **One writer at a time**: an install runs on a worker while the person can
  switch or remove other extras, and several Concat windows (each its own
  process) can share the folder. Every read, change and write of
  `installed.json`, and every move into or out of `packages/`, happens under
  an exclusive lock on `extras.lock` (an OS file lock, so it holds between
  threads and between processes). An install checks its dependencies again
  under the lock as it goes in, so a dependency removed while it unpacked
  stops it. The lock is never held across a download or an unpack. A test
  runs installs and switches on two threads at once. With the lock disabled
  it fails every time; with the lock it passes.
- **Several Concats**: each running window holds a shared lock on
  `running.lock`. A window that starts sweeps `staging/` and `trash/` only
  when it can take that lock alone, so it never deletes or "recovers" an
  install another window is in the middle of. The leftovers wait for a start
  with the folder to itself. This is tested.

## What installed content can do

- **Effects** go through the same pipeline as a hand-made effect package:
  - The manifest and shader are parsed and compiled at install.
  - FFmpeg chains that name file-touching filters are refused.
  - In the window, every shader is trial-run on a separate GPU compositor
    against a timeout before it is offered.
  - Ids under `concat.`, and ids another package already answers to, are
    refused.
  - Headless tools (`concat-cli api`/`serve`/`preview`, MCP) load the same
    packages with the same parsing and compilation, but have no GPU trial.
    See the limitations in EXTRAS_IMPLEMENTATION.md.
- **Fonts** must parse as fonts.
- **Images** must start with the magic bytes their extension claims.
- **Placed stickers and sounds are copied into the project** before they are
  used, so a project never points into an extra's folder that a removal would
  delete.
- **No native code.** Concat links its AI runtimes into the build, and
  extras never load libraries. A future executable-extension system would
  need a separate, permissioned, sandboxed design. It must not be added to
  this format.

## Network and privacy

- HTTPS only, apart from `file:` for a repository on the local disk.
- No account, no token and no telemetry.
- Indexes are cached for six hours. Extras that are already installed keep
  working with no network at all.

## Establishing the official repository (maintainers)

No official key exists yet, and none was invented. Until a maintainer
completes these steps, `OFFICIAL_KEY` is empty, the official repository is
reported as "not set up in this build", and only repositories configured in
`repositories.toml` are used.

1. **Make the key offline.** On a machine kept off the network, run
   `concat-cli extras keygen official-extras.key`. Keep the secret file
   offline (for example on encrypted removable media, with a second copy in a
   separate place). It must never be committed, uploaded to CI, or put on a
   build machine. Record the printed public key. `keygen` never writes over
   an existing file. On Unix it creates the file readable by its owner
   alone; on Windows the file takes its folder's permissions, so make it in
   a folder only you can read.
2. **Compile in the public key.** Set `OFFICIAL_KEY` in
   `src/crates/concat-host/src/extras.rs` to the public key's 64 hex digits.
   Confirm `OFFICIAL_INDEX` is the URL the index will be served from: the
   default is `https://raw.githubusercontent.com/concatapp/assets/main/extras/v1/index.json`,
   beside the sound list. Land this through a reviewed pull request. From that
   release on, the build trusts that key and nothing else for the official
   repository.
3. **Publish.** On the offline machine, run
   `concat-cli extras repo out --key official-extras.key --name "Concat Extras" --valid-days 90 <extra folders…>`.
   Check the result with `concat-cli extras verify out/index.json --public-key <hex>`.
   Upload `index.json`, `index.json.sig` and the `.concatpkg` archives so that
   the archives sit beside the index, or edit their URLs to `https://`
   locations before signing (for example GitHub release assets; never edit
   after signing). The repository must be served over HTTPS.
4. **Re-publish before expiry.** Each publish gets a new serial (the current
   time, unless `--serial` is given) and a new expiry. A missed re-sign makes
   the official repository stop offering anything once the index expires.
   Installed extras keep working.
5. **Rotation and compromise.** There is one official key per build. To
   rotate, ship a release with the new key and publish indexes signed with it.
   Older builds keep using the old key until they update. If the secret is
   compromised, ship a new key in a release at once. Expiry and the serial
   bound how long a stolen key's indexes stay useful against builds that have
   updated. A list of revoked keys is not implemented.

## Known gaps

- There is no revocation list, and no support for two official keys at once
  during a rotation.
- A repository's operator is trusted for the content they sign. The format
  limits what that content can be, not whether it is good.
- The anti-rollback serial is stored on the machine. Someone with write access
  to the user's data folder can delete it, but they can also change the app
  itself.

### Untested assumptions and residual risks (final audit, 2026-11-15)

These were reviewed, and are either accepted or out of the format's reach.
None has a test that would show it holds.

- **The first index is taken on trust (TOFU).** A machine that has never
  seen a repository accepts any index its key signed that has not expired.
  Only expiry bounds how old that first index can be. The serial protects
  every fetch after the first.
- **Expiry trusts the machine's clock.** A clock set back makes an expired
  index look current again.
- **The archive is hashed, then read again to unpack.** Another program
  running as the same user, with write access to `extras/downloads/`, could
  swap the file in that moment. The per-file SHA-256s in the manifest come
  from the swapped file, so they would not catch it. Such a program can
  also edit installed extras, or Concat itself, directly. This is not a
  boundary the format can hold.
- **Parsers see content before it is trusted for use**: TOML (manifests),
  JSON (index, registry), tar and gzip, fonts (the text engine), PNG, JPEG
  and WebP (the media pipeline and Slint), and WGSL (naga). All of them are
  memory-safe Rust crates, apart from FFmpeg, which probes sounds and images
  when they are placed. A malformed file there meets the same decoders as
  any file the person imports by hand. The archive limits (sizes, counts,
  types) apply first.
- **The GPU trial is the window's.** The headless tools compile shaders but
  do not trial them, so a shader that compiles but stalls a GPU is only
  caught in the window.
- **Keys are compared as given.** ring verifies Ed25519 strictly (canonical
  signatures). Concat does not check whether a configured public key is a
  weak point. Keys come from the build or from the person, never from a
  repository.
- **The person decides which repositories to trust.** A person persuaded to
  add a hostile repository to `repositories.toml` gets content signed by
  it. The sheet names the source of every extra, and the format limits the
  content to data.
- **Lock files rely on the file system.** On a network share without
  working byte-range locks, two machines sharing one data folder are not
  kept apart. Concat's data folder is per machine by design.
