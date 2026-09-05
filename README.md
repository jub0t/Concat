<div align="center">

<img src="assets/concat_logo_dark_512.png" alt="Concat" width="140" />

# Concat

**The free, open-source CapCut replacement.**

<p align="center">
  <a href="https://github.com/jub0t/Concat/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/jub0t/Concat/ci.yml?style=flat&logo=githubactions&logoColor=F8F8F8&label=Build&labelColor=000000&color=c6f432" alt="Build Status" /></a>
  <a href="https://github.com/jub0t/Concat/releases/latest"><img src="https://img.shields.io/badge/Download-Cross%E2%80%90Platform-c6f432?style=flat&logo=desktop-download&logoColor=F8F8F8&labelColor=000000" alt="Download Concat" /></a>
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/badge/Version-0.2.1-c6f432?style=flat&logo=semver&logoColor=F8F8F8&labelColor=000000" alt="Concat Version 0.2.1" /></a>
  <a href="https://discord.gg/DVuPfpXfqP"><img src="https://img.shields.io/badge/Discord-Join%20the%20server-c6f432?style=flat&logo=discord&logoColor=F8F8F8&labelColor=000000" alt="Join Concat Discord" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-AGPL%20v3-c6f432?style=flat&logo=gnu&logoColor=F8F8F8&labelColor=000000" alt="License: AGPL-3.0-or-later" /></a>
</p>

<img src="assets/preview-dark.png" alt="Concat editor" width="100%" />

</div>

---

Concat is everything you use CapCut for — without the watermarks, paywalls,
or subscriptions. A native Rust engine does the heavy lifting, a native
interface does the editing, and it all runs on your machine: install it and
start cutting, no account, no extra downloads, no setup.

## Highlights

- Free and local Text-to-Speech features.
- 🎬 Multi-track editing, with several timelines per project when one isn't enough
- ✂️ The cutting toolkit you'd expect: split, trim, merge, transitions, speed control
- 💬 Auto-captions that run entirely on your machine — your audio never leaves it
- 🎙️ Voice filters for cleaning up or playing with your sound
- 📝 Titles and styled text
- 📦 Templates — build an edit once, reuse it for the next video
- 🚫 No watermarks, no account, nothing behind a paywall
- 🖥️ Works the same on macOS, Windows and Linux
- 🌍 Eleven languages, and a new one is a single JSON file — see [TRANSLATING.md](TRANSLATING.md)

## Get started

Concat is currently in **Beta version (pre-release)**. **Download** the latest build from [Releases](https://github.com/jub0t/Concat/releases).

**Platform support:**

- ✅ **Windows** — tested
- ✅ **macOS** — unsigned binaries; run:
  `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux**
  - 🧪 ARM
  - 🧪 x86_64
- 🚧 **Android**
  - Phones
  - Tablets
- 🚧 **iOS / iPadOS**
  - iPhone
  - iPad
- 🧪 **Raspberry Pi 5**

**Status:** ✅ Supported · 🚧 Work in progress · 🧪 To be tested

## Contribution

> [!IMPORTANT]
> The best way to contribute is to grab a build from the [Release](https://github.com/jub0t/Concat/releases) page and test the application to see where it breaks or how it can be improved.

Ready to write code? [CONTRIBUTING.md](./CONTRIBUTING.md) covers setup, layout, the checks to run, and how contributions are licensed. There is also [this Discussion announcement](https://github.com/jub0t/Concat/discussions/3). Read [ROADMAP.MD](./ROADMAP.MD) for future goals.

## License

Concat is free software under the [GNU Affero General Public License v3.0 or
later](./LICENSE).

**If you just want to edit videos, none of this affects you.** Download a build
and use it for anything, including commercial work. There is nothing to accept,
register or pay, and no obligation attaches to you as a user.

**If you fork it, you are welcome here.** Modify it, ship it, sell it, host it.
The one condition is reciprocity: those who receive your version get the source
too, under the same license. That includes users who reach it over a network —
that is the "Affero" part, and it is why a hosted Concat cannot be closed.

Two things sit alongside the AGPL:

- **Plugins keep their own license.** The [plugin
  exception](./LICENSE-EXCEPTIONS.md) means anything built on the Concat API is
  yours to license as you like. The copyleft covers Concat, not what talks to it.
- **The name is not part of the grant.** The code is free; *Concat* and the logo
  identify builds from this project. Forks are encouraged — please ship them
  under your own name. See [TRADEMARK.md](./TRADEMARK.md).

Contributions are made under the [CLA](./CLA.md); you keep copyright in your
work. Distributed builds also include GPL-licensed FFmpeg and espeak-ng — see
[THIRD_PARTY_NOTICES.md](./THIRD_PARTY_NOTICES.md).

Need terms without the AGPL's obligations? A commercial license is available:
**jub0trd@gmail.com**.
