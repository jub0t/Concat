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
- 🌍 Twelve languages, and a new one is a single JSON file — see [TRANSLATING.md](TRANSLATING.md)

## Concat vs CapCut vs OpenCut

The short version: CapCut has the most features and the worst terms, OpenCut has the biggest community and the least finished editor, Concat has the best terms and a real native engine, but it is a beta with gaps it has not closed yet.

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
| Keyframes | 🔴 | 🟢 | 🟡 | Concat has none yet. On the roadmap |
| Export formats | 🟡 | 🟢 | 🟡 | Concat writes H.264 MP4 only. OpenCut MP4 and WebM |
| Stability | 🟡 | 🟢 | 🔴 | Concat is a 0.2.x beta. OpenCut is mid rewrite |
| Mobile | 🟡 | 🟢 | 🔴 | Concat's Android and iOS builds compile but are untested. OpenCut's are in progress |
| Extensibility | 🟡 | 🔴 | 🟢 | OpenCut ships an Editor API, MCP server and plugins. Concat's plugin API is planned |
| Community | 🟡 | 🟢 | 🟢 | OpenCut has tens of thousands of stars. Concat has a Discord and a handful of contributors |
| Multiple timelines per project | 🟢 | 🔴 | 🔴 | Concat only |

<details>
<summary><strong>Full breakdown</strong></summary>

| | **Concat** | **CapCut** | **OpenCut** |
|---|---|---|---|
| Price | Free, no tiers | Free tier; Standard $9.99/mo, Pro $19.99/mo or $179.99/yr | Free |
| Watermark | Never | On Pro templates and effects unless you pay | Never |
| 4K export | Yes, free | Pro only (free caps at 1080p) | Depends on the browser and machine |
| Source code | Open, AGPL-3.0 | Closed | Open, MIT |
| Who owns your footage | You. Nothing leaves your machine | You grant ByteDance a perpetual, irrevocable, sublicensable licence to anything you upload, including face and voice data (ToS, June 2025) | You. Processed locally |
| Account required | No | Yes, for most features | No |
| Runs as | One native Rust binary, FFmpeg linked in | Native apps plus a heavy cloud dependency | Web app first; desktop and mobile are a 2026 rewrite in progress |
| Rendering | CPU and GPU (wgpu) compositors, same maths in preview and export | Native, GPU accelerated, mature | Browser stack (WebAssembly FFmpeg); slower on long or heavy timelines |
| Auto captions | On device, whisper.cpp compiled in | Cloud, fast, very good, feeds the ToS above | Transcription service, less integrated |
| Text to speech | On device, Kokoro, free | Cloud, many voices, partly behind Pro | Not a headline feature |
| Background removal | Automatic person cutout, brush cleanup, green and blue screen, all on device | Auto, custom brush, chroma key. Best in class | Via a separate Python service |
| Effects and filters | About 80 built in, data driven packages, WGSL shader support | Thousands, plus a marketplace | Basic set |
| Transitions | Basic and motion sets | Hundreds | Basic set |
| Keyframe animation | Not yet, on the roadmap | Yes, mature | Partial |
| Templates | Yes, local | Huge community library | Early |
| Stock music and media | No | Large library (licence terms apply) | No |
| Export formats | MP4 (H.264 + AAC) only | MP4, MOV, GIF, more | MP4, WebM |
| Multiple timelines per project | Yes | No | No |
| Languages | 12, add one with a JSON file | 20+ | Fewer |
| Plugins and scripting | Planned (Concat API, plugin exception in the licence) | No | Editor API, MCP server, plugins |
| Mobile | Android and iOS builds exist, untested | Excellent, where CapCut started | In progress |
| Maturity | Beta, 0.2.1 | Production, hundreds of millions of users | Alpha-grade editor with a very large GitHub following |

</details>

### Where Concat loses, honestly

- **CapCut has far more content.** Thousands of effects, transitions, stickers, fonts, templates and stock tracks against Concat's few dozen of each. If your workflow is "pick a trending template and swap the clips", CapCut is still the tool.
- **No keyframes yet.** You cannot animate position, scale or an effect parameter over time. This is the biggest missing feature and it is on the roadmap.
- **One export format.** H.264 MP4 with AAC. No MOV, no ProRes, no GIF, no HEVC yet.
- **It is a beta.** Things break. macOS builds are unsigned and need a `xattr` command to open. Mobile builds compile but nobody has tested them properly. Expect rough edges that a product with ByteDance's budget does not have.
- **Speech models download on first use.** Captions and text to speech pull their models the first time you run them. After that they are fully offline, but the first run is not "zero downloads".
- **AGPL is not for everyone.** Concat's code is copyleft. Plugins are exempt (see [LICENSE-EXCEPTIONS.md](LICENSE-EXCEPTIONS.md)) but if you want to embed the engine in a closed product, OpenCut's MIT licence is friendlier.
- **The community is small.** OpenCut has tens of thousands of stars and a developer ecosystem around its Editor API. Concat has a Discord and a handful of contributors. Bugs get fixed, but not at CapCut's pace.
- **Fewer tracked and AI features.** No object tracking, no masks that follow a subject, no auto reframe, no beat sync, no filler word removal. All of these are on the roadmap, none of them ship today.

### Who should pick what

- **Pick CapCut** if you need the template library, the mobile experience, or the latest AI toys, and you are fine with the terms of service.
- **Pick OpenCut** if you want an MIT-licensed editor to build on, or you live in the browser and never edit anything heavy.
- **Pick Concat** if you want CapCut's core workflow, offline, free, with your footage staying yours, and you can live with a beta that is still filling in the long tail.

## Get started

Concat is currently in **Beta version (pre-release)**. **Download** the latest build from [Releases](https://github.com/jub0t/Concat/releases).

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

## Contribution

> [!IMPORTANT]
> The best way to contribute is to grab a build from the [Release](https://github.com/jub0t/Concat/releases) page and test the application to see where it breaks or how it can be improved.

Ready to write code? [CONTRIBUTING.md](./CONTRIBUTING.md) covers setup, layout, the checks to run, and how contributions are licensed. There is also [this Discussion announcement](https://github.com/jub0t/Concat/discussions/3). Read [ROADMAP.MD](./ROADMAP.MD) for future goals.
