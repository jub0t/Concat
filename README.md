<div align="center">
<table width="100%">
  <tr>
    <td align="left" width="120">
      <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/logo-dark.png" alt="Concat" width="100" />
    </td>
    <td align="right">
      <h1>Concat</h1>
      <h3 style="margin-top: -10px;">The truly free, and open-source cross-platform CapCut replacement.</h3>
    </td>
  </tr>
</table>

<p align="center">
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/github/downloads/jub0t/concat/total?style=flat&logo=github&logoColor=F8F8F8&label=Downloads&labelColor=000000&color=c6f432" alt="Total Downloads" /></a>
  <a href="https://github.com/jub0t/Concat/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/jub0t/Concat/ci.yml?style=flat&logo=githubactions&logoColor=F8F8F8&label=Build&labelColor=000000" alt="Build Status" /></a>
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/badge/Version-0.2.3-c6f432?style=flat&logo=semver&logoColor=F8F8F8&labelColor=000000" alt="Concat Version 0.2.3" /></a>
  <a href="https://discord.gg/DVuPfpXfqP"><img src="https://img.shields.io/badge/Discord-Join%20the%20server-5865F2?style=flat&logo=discord&logoColor=F8F8F8&labelColor=000000" alt="Join Concat Discord" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-AGPL%20v3-c6f432?style=flat&logo=gnu&logoColor=F8F8F8&labelColor=000000" alt="License: AGPL-3.0-or-later" /></a>
</p>

<img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/editor.png" alt="Concat editor" width="100%" />

</div>

---

## About

Concat is everything you use CapCut for. No watermarks. No paywalls. No subscriptions.

It runs entirely on your machine, powered by a native Rust engine. Install it and start cutting. No account, no setup.

## Highlights

- 🚫 **No watermarks.** No account. No paywall.
- 🔒 **100% local.** Nothing leaves your machine.
- 🎬 **Multi-track editing.** Several timelines per project.
- ✂️ **Cut fast.** Split, trim, merge, transitions, speed control.
- 💬 **Auto-captions.** Runs on your machine, offline.
- 🗣️ **Text-to-Speech.** Free, local voices.
- 🎙️ **Voice filters.** Clean up or play with your sound.
- 📝 **Titles and styled text.**
- 📦 **Templates.** Build an edit once, reuse it.
- 🖥️ **macOS, Windows and Linux.** Same app everywhere.
- 🌍 **Fourteen languages.** Add one with a single JSON file, see [TRANSLATING.md](TRANSLATING.md).

## Get started

Concat is currently in **Beta version (pre-release)**. **Download** the latest build from [Releases](https://github.com/jub0t/Concat/releases).

**Reporting something:** every run writes a log, and Settings › About has the button that opens it along with the one that copies your system information. Attach both to an [issue](https://github.com/jub0t/Concat/issues) and the report arrives with everything it needs. The last ten runs are kept, so yesterday's is still there; nothing is ever sent anywhere on its own.

**Platform support:**

- ✅ **Windows**
  - ✅ x86_64
- ✅ **macOS** — unsigned binaries; run:
  `xattr -dr com.apple.quarantine /Applications/Concat.app`
  - ✅ Intel
  - ✅ Silicon
- ✅ **Linux**
  - ✅ ARM
  - ✅ x86_64
- ✅ **Android**
  - ✅ Phones
  - ✅ Tablets
- 🧪 **iOS / iPadOS**
  - 🧪 iPhone
  - 🧪 iPad

**Status:** ✅ Supported · 🚧 Work in progress · 🧪 To be tested

**System requirements:**

Concat runs everything on your machine, so the hardware sets the ceiling. The minimum column is what a build will run on at all; the recommended column is what makes 1080p editing feel smooth and keeps 4K exports and captions from being a wait.

| | Minimum | Recommended |
|---|---|---|
| **CPU** | Any 64-bit processor from 2013 or later | 6 cores or more |
| **GPU** | None. Without a usable GPU the window and monitor fall back to the CPU | Any GPU with Metal (macOS), DirectX 12 (Windows) or Vulkan (Linux) |
| **RAM** | **4 GB** | **16 GB** for 4K timelines and the larger caption models |
| **Storage** | **500 MB** for the app and the smallest caption model | **2 GB** for every optional model, plus room for projects and exports |

Optional models download from the settings panel on first use and then never need the network again: auto-captions 78 MB to 488 MB depending on the whisper size you pick, text-to-speech 132 MB or 349 MB, person cutout 15 MB, object cutout 179 MB, and the cutout brush 40 MB.

## How to Contribute

> [!IMPORTANT]
> The best way to contribute is to grab a build from the [Releases](https://github.com/jub0t/Concat/releases) page and use it: find where it breaks, and say where it could be better.
>
> Ready to write code? [CONTRIBUTING.md](./CONTRIBUTING.md) covers setup, the layout of the tree, the checks to run, and how contributions are licensed. Driving Concat from a script, a service or an agent? [docs/](./docs/README.md) is the developer reference for the Concat API and its transports: JSON-RPC, gRPC and MCP. [This Discussion](https://github.com/jub0t/Concat/discussions/3) is where the project was announced.
> 
> Contributors are free to claim a `@Contributor` role in the Discord server, just ask for it.

## Contributors

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## Star History

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=timeline&legend=top-left">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&theme=dark&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=top-left" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=top-left" />
 </picture>
</a>
