# Arch Linux packaging

`makepkg -si` from this directory builds and installs WolfCut.

The package is `wolfcut-git`, not a versioned `wolfcut`. Every green build on
`main` mints a fresh `v<version>-alpha.<n>` tag, so a pinned release would be
stale within a day; `pkgver()` reads the newest tag out of the checkout
instead.

## Why a native package and not the AppImage

The AppImage is built on Ubuntu 22.04 and carries that machine's GTK stack,
`libwayland-client` included. On a rolling distro the system Mesa is far newer
than that library, EGL display creation fails with `EGL_BAD_PARAMETER`, the
WebKit web process dies, and the editor comes up as an empty window.

This package links the system WebKitGTK, Wayland and Mesa, so there is no
version skew to hit.

## Bundled tools

`build.rs` refuses a release build without a staged ffmpeg/ffprobe/whisper-cli
trio, because a desktop launch has no `PATH` to fall back on. A distro package
must not ship private copies of what the repos already provide, so the build
sets `WOLFCUT_SYSTEM_TOOLS=1` to opt out - the same deal `flake.nix` makes -
and depends on the system `ffmpeg`. The app looks for its tools beside the
executable first and falls back to `PATH`.

`whisper.cpp` is an `optdepends`: without it everything but transcription
works, and Settings > Transcriber can still be pointed at a `whisper-cli`
elsewhere.
