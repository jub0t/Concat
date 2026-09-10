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
  <a href="https://github.com/jub0t/Concat/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/jub0t/Concat/ci.yml?style=flat&logo=githubactions&logoColor=F8F8F8&label=Build&labelColor=000000" alt="Build Status" /></a>
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/github/downloads/jub0t/concat/total?style=flat&logo=github&logoColor=F8F8F8&label=Downloads&labelColor=000000&color=c6f432" alt="Total Downloads" /></a>
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/badge/Version-0.2.1-c6f432?style=flat&logo=semver&logoColor=F8F8F8&labelColor=000000" alt="Concat Version 0.2.1" /></a>
  <a href="https://discord.gg/DVuPfpXfqP"><img src="https://img.shields.io/badge/Discord-Join%20the%20server-c6f432?style=flat&logo=discord&logoColor=F8F8F8&labelColor=000000" alt="Join Concat Discord" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-AGPL%20v3-c6f432?style=flat&logo=gnu&logoColor=F8F8F8&labelColor=000000" alt="License: AGPL-3.0-or-later" /></a>
</p>

<img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/editor.png" alt="Concat editor" width="100%" />

</div>

---

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
- 🌍 **Twelve languages.** Add one with a single JSON file, see [TRANSLATING.md](TRANSLATING.md).

## Get started

Concat is currently in **Beta version (pre-release)**. **Download** the latest build from [Releases](https://github.com/jub0t/Concat/releases).

**Portable:** the Windows and Linux builds are plain archives. To keep everything on the stick or in the folder you unpacked into, make a folder named `portable` beside the `concat` executable: settings, recents and downloaded models then live there and nothing is written to the user profile.

**Platform support:**

- ✅ **Windows** — tested
- ✅ **macOS** — unsigned binaries; run:
  `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux**
  - 🧪 ARM
  - 🧪 x86_64
- 🧪 **Android**
  - Phones
  - Tablets
- 🧪 **iOS / iPadOS**
  - iPhone
  - iPad

**Status:** ✅ Supported · 🚧 Work in progress · 🧪 To be tested

**System requirements:**

Concat runs everything on your machine, so the hardware sets the ceiling. The minimum column is what a build will run on at all; the recommended column is what makes 1080p editing feel smooth and keeps 4K exports and captions from being a wait.

| | Minimum | Recommended |
|---|---|---|
| **CPU** | Any 64-bit processor from 2013 or later | 6 cores or more |
| **GPU** | None. Without a usable GPU the window and monitor fall back to the CPU | Any GPU with Metal (macOS), DirectX 12 (Windows) or Vulkan (Linux) |
| **RAM** | 4 GB | 16 GB for 4K timelines and the larger caption models |
| **Storage** | 500 MB for the app and the smallest caption model | 2 GB for every optional model, plus room for projects and exports |
| **Display** | 1280 × 720 | 1920 × 1080 or larger |
| **Audio** | Optional. Playback wants an output device, everything else works without one | |
| **Internet** | Not required. Only used when you download an optional model, once | |

Optional models download from the settings panel on first use and then never need the network again: auto-captions 78 MB to 488 MB depending on the whisper size you pick, text-to-speech 132 MB or 349 MB, person cutout 15 MB, object cutout 179 MB, and the cutout brush 40 MB.

## Contribution

> [!IMPORTANT]
> The best way to contribute is to grab a build from the [Release](https://github.com/jub0t/Concat/releases) page and test the application to see where it breaks or how it can be improved.

Ready to write code? [CONTRIBUTING.md](./CONTRIBUTING.md) covers setup, layout, the checks to run, and how contributions are licensed. There is also [this Discussion announcement](https://github.com/jub0t/Concat/discussions/3). Read [ROADMAP.MD](./ROADMAP.MD) for future goals.

## Concat vs CapCut vs OpenCut

🟢 strong · 🟡 partial or with strings attached · 🔴 weak or missing

| | Concat | CapCut | OpenCut | Notes |
|---|:---:|:---:|:---:|---|
| Performance | 🟢 | 🟢 | 🟡 | Concat and CapCut are native. OpenCut runs on WebAssembly FFmpeg in a browser |
| Price | 🟢 | 🟡 | 🟢 | CapCut is free until Pro effects, 4K or AI tools, then $9.99 to $19.99 a month |
| Watermark | 🟢 | 🟡 | 🟢 | CapCut stamps exports that use Pro assets |
| Privacy | 🟢 | 🔴 | 🟢 | Concat sends nothing anywhere. CapCut's terms grant ByteDance a perpetual licence to uploads |
| Offline | 🟢 | 🟡 | 🟡 | Concat's captions, speech, cutout and export all run on device. CapCut's best features are cloud |
| Open source | 🟢 | 🔴 | 🟢 | Concat AGPL, OpenCut MIT, CapCut closed |
| 4K export | 🟢 | 🟡 | 🟡 | CapCut caps free at 1080p. OpenCut depends on the browser |
| Effects and templates | 🟡 | 🟢 | 🔴 | CapCut has thousands. Concat has a few dozen. OpenCut has a basic set |
| AI tools | 🟡 | 🟢 | 🟡 | CapCut has tracking, reframe, avatars. Concat has local captions, speech and person cutout |
| Keyframes | 🟡 | 🟢 | 🟡 | Concat keys position, scale, rotation and opacity with bezier easing. No curve editor and no keyed effect parameters yet |
| Export formats | 🟡 | 🟢 | 🟡 | Concat writes H.264 MP4 only. OpenCut MP4 and WebM |
| Stability | 🟡 | 🟢 | 🔴 | Concat is a 0.2.x beta. OpenCut is mid rewrite |
| Mobile | 🟡 | 🟢 | 🔴 | Concat's Android and iOS builds compile but are untested. OpenCut's are in progress |
| Extensibility | 🟡 | 🔴 | 🟢 | OpenCut ships an Editor API, MCP server and plugins. Concat's plugin API is planned |
| Community | 🟡 | 🟢 | 🟢 | OpenCut has tens of thousands of stars. Concat has a Discord and a handful of contributors |
| Multiple timelines per project | 🟢 | 🔴 | 🔴 | Concat only |
