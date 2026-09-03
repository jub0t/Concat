# Contributing to Concat

Thanks for being here. Concat is a video editor that runs entirely on the
user's machine, and it gets better mostly through people using it hard and
reporting what broke.

## The most valuable thing you can do

Grab a build from the [Releases](https://github.com/jub0t/Concat/releases) page
and edit a real video with it. Alpha software fails in ways nobody predicts
from reading the source. A good bug report — what you did, what happened, your
OS, the media you used — is worth more than most patches.

Longer-form discussion lives in
[the contribution discussion](https://github.com/jub0t/Concat/discussions/3)
and on [Discord](https://discord.gg/DVuPfpXfqP).

## Before you write code

Open an issue or drop into Discord first for anything beyond a small fix. Large
areas of [`ROADMAP.MD`](ROADMAP.MD) are already in progress or intentionally
deferred, and it is genuinely no fun to review a big PR that has to be turned
down for reasons that were invisible from outside.

## Setting up

The repo is a Nix flake, which is the shortest path to a working toolchain:

```sh
nix develop        # shell with ffmpeg, whisper and the Rust/Node toolchains
npm run app        # run the editor
```

Without Nix you will need Rust (see `rust-version` in `engine/Cargo.toml`)
and `ffmpeg`/`ffprobe` on `PATH`. Then:

```sh
cd engine && cargo run -p concat
```

That is the Slint editor window, which is where new UI work goes. The Tauri +
React app in `desktop/` is deprecated: it is what releases still ship until
the Slint window can do everything it did, so bug fixes there are welcome,
but features are not. Running it additionally needs Node:

```sh
cd desktop && npm install && npm run app
```

## Layout

| Path | What lives there |
|---|---|
| `engine/crates/` | The Rust engine — core, media, render, export, project, cli — and `concat`, the Slint editor window |
| `desktop/src/` | The React editor UI (deprecated) |
| `desktop/src-tauri/` | Tauri host: commands, session lifecycle, IPC (deprecated) |
| `test/` | Media and analysis fixtures |

[`ARCHITECTURE.md`](ARCHITECTURE.md) explains how these fit together and where
the sharp edges are. Read it before touching the engine.

## Checks

Run these before opening a PR:

```sh
cd engine  && cargo fmt --check && cargo clippy --all-targets && cargo test
cd desktop && npm run typecheck && npm run lint && npm test   # only if you touched desktop/
```

New source files need a licence header — see below. Match the style of the code
around you; the engine avoids cleverness on purpose.

## Licensing your contribution

Concat is **AGPL-3.0-or-later** ([`LICENSE`](LICENSE)), with a plugin exception
so that Concat API plugins can carry their own licence
([`LICENSE-EXCEPTIONS.md`](LICENSE-EXCEPTIONS.md)).

Contributions are accepted under the CLA in [`CLA.md`](CLA.md). **You keep the
copyright in your work** — the CLA is a licence, not an assignment. It exists so
that copyright in Concat stays centralised, which is what makes the AGPL
enforceable against companies that take the code without honouring it.

Sign off your commits to indicate agreement:

```sh
git commit -s -m "your message"
```

Typo and documentation fixes do not need a sign-off.

### File headers

Every source file starts with:

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors
```

Keep your own copyright line if you want one — add it, don't replace what's
there.

Generated files are exempt: `desktop/src/lib/generated/` is ts-rs output and is
rewritten wholesale, so headers there would not survive. Those files are covered
by the repository licence like everything else.

### Third-party code

If your patch brings in code you did not write, say so in the PR: what it is,
where it came from, and its licence. Anything incompatible with
AGPL-3.0-or-later cannot be merged, and licence problems are much cheaper to
catch before a merge than after a release.

## Trademarks

The code is free to fork. The Concat name and logo are not covered by the AGPL
grant — see [`TRADEMARK.md`](TRADEMARK.md). Forks are welcome; please ship them
under your own name.

## Conduct

[`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md) applies to the repo, the Discussions
and Discord.
