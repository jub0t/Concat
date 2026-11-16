# Development environment (Windows, project-local)

This is how this checkout was built and tested on Windows with every new
tool, cache and temporary file kept in one folder beside the repository,
`NeededDownloadsAndInstall`. Nothing was installed machine-wide, and no
global environment variable or registry entry was changed.

```
CutCap\
  ConCut\                       this repository
  NeededDownloadsAndInstall\    every development download, runtime, cache and build output
    scripts\env.sh              the environment (source it from Git bash)
    scripts\dl.sh               downloads one file and refuses it unless its SHA-256 matches
    manifests\                  installed_tools.json, downloaded_files.json, environment_configuration.json
```

## What is needed

| Tool | Version | Where | Why |
|---|---|---|---|
| Rust (rustup, cargo, rustfmt, clippy) | 1.93.1, pinned by `src/rust-toolchain.toml` | `runtimes\rust\rustup`, `package_cache\cargo` | the workspace |
| MSVC compiler and linker | 14.52 (Visual Studio 2026 Community) | existing install, **reused** | the `x86_64-pc-windows-msvc` target, whisper.cpp |
| Windows SDK + CRT headers and libraries | SDK 10.0.26100 | `sdk\windows\xwin`, fetched with **xwin 0.10.0** | linking. There was no SDK installed, so xwin downloads Microsoft's own packages into a folder instead (Microsoft's licence applies) |
| LLVM | 21.1.0 (as `scripts/ci/deps.lock`) | `runtimes\other\llvm-21.1.0`, selected files | `libclang` for bindgen; `llvm-rc`/`llvm-mt` stand in for the SDK's `rc.exe`/`mt.exe` |
| CMake, Ninja | 4.5.0, 1.13.2 | `runtimes\other` | whisper.cpp's build. Ninja, because a Visual Studio generator wants a registered SDK |
| FFmpeg dev (BtbN LGPL shared) | 8.1, as `deps.lock` | `runtimes\other\ffmpeg-8.1-win64-lgpl-shared` | `FFMPEG_DIR` |
| ONNX Runtime | 1.28.0, as `deps.lock` | `runtimes\other\onnxruntime-win-x64-1.28.0` | `ORT_LIB_LOCATION`, the way CI links it |
| Python (embeddable) | 3.14.10 | `runtimes\python\python-3.14` | `scripts/locales.py --check` |
| Git | existing | reused | |

Every archive was checked against its published SHA-256 (rustup's `.sha256`,
GitHub's asset digests, CMake's `SHA-256.txt`, python.org's `sha256_sum`, and
`scripts/ci/deps.lock` for FFmpeg and ONNX Runtime).

## Setting it up again

From Git bash, with the folders above in place:

```sh
D="/d/Claude Project/CutCap/NeededDownloadsAndInstall"
export TEMP="$D/temp" TMP="$D/temp" RUSTUP_HOME="$D/runtimes/rust/rustup" CARGO_HOME="$D/package_cache/cargo"
"$D/downloads/installers/rustup-init.exe" -y --no-modify-path --profile minimal \
    --default-host x86_64-pc-windows-msvc --default-toolchain 1.93.1 -c rustfmt -c clippy
"$D/runtimes/other/xwin-0.10.0-x86_64-pc-windows-msvc/xwin.exe" --accept-license --arch x86_64 \
    --cache-dir "$D/downloads/source_packages/xwin-cache" \
    splat --use-winsysroot-style --include-debug-libs --output "$D/sdk/windows/xwin"
```

`--use-winsysroot-style` avoids a symlink that needs administrator rights.
`--include-debug-libs` adds `ucrtd.lib` and the other debug CRT libraries,
which CMake's debug try-compile links against.

## Building and testing

```sh
. "/d/Claude Project/CutCap/NeededDownloadsAndInstall/scripts/env.sh"
cd "/d/Claude Project/CutCap/ConCut/src"
cargo test -p concat-extras -p concat-effects -p concat-host -p concat-cli
cargo clippy -p concat-extras -p concat-host -p concat-effects -p concat-cli --all-targets -- -D warnings
cargo check -p concat --lib -j 6        # the window: its generated UI needs ~7.5 GB of RAM per rustc
cargo build -p concat --profile quick   # an optimised window binary that rebuilds quickly
python scripts/locales.py --check       # from the repository root, with the embeddable Python
```

`env.sh` sets everything the build reads:

* `TEMP`, `TMP`, `CARGO_HOME`, `RUSTUP_HOME` and `CARGO_TARGET_DIR` all point
  inside `NeededDownloadsAndInstall`.
* `VCINSTALLDIR`, `INCLUDE` and `LIB` work like a developer prompt: MSVC
  from Visual Studio, and SDK headers and libraries from xwin. cc-rs, CMake
  and rustc then take tools from `PATH` and never search the registry.
* `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER` points at MSVC's `link.exe`.
  Git bash has an unrelated `/usr/bin/link.exe` that would otherwise be used.
* `LIBCLANG_PATH`, `RC_PATH` (the exe icon, via winresource),
  `CMAKE_GENERATOR=Ninja`, `FFMPEG_DIR`, `ORT_LIB_LOCATION` and
  `ORT_PREFER_DYNAMIC_LINK=1` cover the native dependencies.

## Unavoidable system pieces

* The **MSVC compiler** comes from the Visual Studio 2026 Community install
  already on C:. It was reused, not modified.
* Windows itself keeps caches on C: when new programs run (prefetch,
  Defender scans). These are outside what a build can redirect.
* `rustup` warns that MSVC "prerequisites" are missing, because no SDK is
  registered. That is expected here, since the SDK comes from xwin.

## Known test failures that are not from Extras

On this machine, three `concat-host` export integration tests
(`one_clip_exports_whole_at_every_rate`, `the_edges_export_too`,
`every_edit_still_exports`) failed before any Extras change, with the audio
track a few tens of milliseconds short (for example "the sound runs 5.931s,
not 6s"). CI calls `every_edit_still_exports` a known Windows trouble spot.
These tests are unrelated to Extras and were left as they are.
