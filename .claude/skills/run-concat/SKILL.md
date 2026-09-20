---
name: run-concat
description: Build, launch, and verify the Concat desktop app after a code change. Use this whenever the user asks to run Concat, test a change in the app, check whether something works, see the editor, or verify a fix "in the real app" rather than just via tests — even if they don't say "run the app" explicitly (e.g. "does this look right now", "let's see it"). Concat is a native Rust + Slint desktop video editor with a Cargo workspace root at src/, not the repo root, which trips up plain `cargo run`.
---

# Running and verifying Concat

Concat is a native Rust + Slint desktop app. The git repo root and the Cargo workspace root are **different directories** — this is the single most common way a run fails.

## Repo layout

- Git repo root: wherever `.git` lives (e.g. `/Users/maubrick/Documents/Coding Projects/Concat`).
- Cargo workspace root: `<repo>/src` — this is where `Cargo.toml` for the workspace lives. **Every `cargo` command must run from here**, not the repo root.

If a `cargo` command fails with something like `no such file or directory` for a path that looks otherwise correct, the most likely cause is running from the repo root instead of `src/`. Fix with `cd` into `src/` (use the absolute path — don't assume the shell's prior `cd` state).

## Building and running

```bash
cd "<repo-root>/src"
cargo run --profile quick -p concat      # optimized build, much faster than a full release build — the default choice for iteration
cargo run -p concat                       # plain debug build if `quick` profile is unavailable for some reason
```

The `quick` profile build of the whole workspace typically takes a few minutes the first time (or after touching a widely-depended-on crate like `concat-core` or `concat-effects`) and is fast on incremental rebuilds. Don't assume a hang — a multi-minute compile is normal for this codebase's size.

This launches a real GUI window (Slint desktop app on macOS/Windows/Linux). It is **not** a web app and **not** an iOS simulator target — don't reach for the browser tools or the iOS simulator tools for it. There is no dedicated MCP or CLI-driven UI automation for this app in a typical session; verification is primarily:

1. **Does it build and launch without error.** Read the terminal it's running in (prefer the terminal MCP's `read_terminal` over asking the user to paste output, if a terminal tool is available) and check for a clean startup log line (e.g. `INFO [main] concat_host::logs: Concat <version> · <platform> · INFO`) with no panics or error-level lines after it.
2. **Ask the user to look.** Since there's no headless way to drive this specific GUI, once it's running, tell the user what to check and let them report back — don't claim a UI behavior works without either seeing it yourself (if a computer-use tool is available and the user has granted it) or being told so by the user. This matches the project's general rule: type-checking and test suites verify code correctness, not feature correctness.
3. **If a computer-use tool is available and the user wants a screenshot**, it's a normal macOS window — screenshot/click it like any other native app, not through the browser or simulator tools.

## Verification commands (run these first — cheaper and faster than launching the GUI)

```bash
cd "<repo-root>/src"

# Full test suite for the crates most likely to be touched by an effects/transitions/rendering change
cargo test -p concat-effects -p concat-export -p concat-render -p concat-project --features gpu

# Full app type-check, including the Slint UI macro — catches .slint wiring errors cargo test alone won't
cargo check -p concat

# Lint
cargo clippy -p concat -p concat-export -p concat-effects -p concat-render -p concat-project \
  --features concat-render/gpu -- -D warnings
```

Run `cargo check -p concat` even for changes that look pure-Rust or pure-`.slint` — the Slint build macro re-generates Rust bindings from the `.slint` files at compile time, so a mismatch between a `.slint` property/callback and its Rust wiring only surfaces here, not in a narrower per-crate `cargo check`.

The `--features gpu` tests actually create a real GPU pipeline (via wgpu) and run shaders on it — this is the only thing that catches a bind-group-layout or binding mistake in a `.wgsl` shader; plain compilation of the shader (which `cargo test -p concat-effects` alone also does, via naga validation) only catches syntax and type errors, not pipeline-creation errors. Don't skip the GPU tests for a shader change just because the plain build succeeded.

## Package system note (if the change touches effects/filters/transitions)

Concat auto-discovers every effect/filter/transition/audio package from a folder under `src/crates/concat-effects/packages/concat.<name>/` at compile time (`build.rs`, via `include_str!`) — a new package needs no Rust code to register it, just the folder. If a change there doesn't show up after a rebuild, check the folder was actually picked up (`cargo clean -p concat-effects` forces `build.rs` to rerun if `cargo`'s change-detection seems stale) before assuming the change itself is wrong.

## Quick checklist for "does this change actually work"

1. Run the cheap verification commands above first — most bugs are caught here, long before a GUI launch is worth the time.
2. `cd` into `src/` with an absolute path, then `cargo run --profile quick -p concat`.
3. Read the terminal for a clean startup and no error-level log lines.
4. Tell the user specifically what to click/check for the change in question, rather than a generic "let me know how it looks" — point at the exact panel, tab, or clip state that exercises the change.
