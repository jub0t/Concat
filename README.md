<div align="center">

<img src="assets/concat_logo_dark_512.png" alt="Concat" width="140" />

# Concat

**The free, open-source CapCut replacement.**

<p align="center">
  <a href="https://github.com/jub0t/Concat/actions/workflows/build.yml"><img src="https://img.shields.io/github/actions/workflow/status/jub0t/Concat/build.yml?style=flat&logo=githubactions&logoColor=F8F8F8&label=Build&labelColor=000000&color=161616" alt="Build Status" /></a>
  <a href="https://github.com/jub0t/Concat/releases/latest"><img src="https://img.shields.io/badge/Download-macOS%20%7C%20Windows%20%7C%20Linux-161616?style=flat&logo=desktop-download&logoColor=F8F8F8&labelColor=000000" alt="Download Concat" /></a>
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/badge/Version-0.2.0-161616?style=flat&logo=semver&logoColor=F8F8F8&labelColor=000000" alt="Concat Version 0.2.0" /></a>
  <a href="https://discord.gg/DVuPfpXfqP"><img src="https://img.shields.io/badge/Discord-Join%20the%20server-161616?style=flat&logo=discord&logoColor=F8F8F8&labelColor=000000" alt="Join Concat Discord" /></a>
</p>

<img src="assets/preview-dark.png" alt="Concat editor" width="100%" />

</div>

---

<a href="https://trendshift.io/repositories/191379?utm_source=trendshift-badge&amp;utm_medium=badge&amp;utm_campaign=badge-trendshift-191379" target="_blank" rel="noopener noreferrer"><img src="https://trendshift.io/api/badge/trendshift/repositories/191379/daily?language=TypeScript" alt="jub0t%2FConcat | Trendshift" width="250" height="55"/></a>

Concat is everything you use CapCut for — without the watermarks, paywalls,
or subscriptions. A native Rust engine does the heavy lifting, a clean React
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

## Get started

Currently in Alpha (pre-release), Download from [Releases](https://github.com/jub0t/Concat/releases), Supports:
- Windows (tested)
- MacOs (tested) - unsigned binaries, use `xattr -dr com.apple.quarantine /Applications/Concat.app`
- Linux

### Nix (Linux)

The repository is a flake. `nix run github:jub0t/Concat` starts the editor
with ffmpeg and whisper wired in; `nix develop` opens a shell with everything
`npm run app` needs.

## Contribution

> [!IMPORTANT]
> The best way to contribute is to grab a build from the [Release](https://github.com/jub0t/Concat/releases) page and test the application to see where it breaks or how it can be improved.

To learn more about contributing to this project please refer to [this Discussion announcement](https://github.com/jub0t/Concat/discussions/3).

## Roadmap

Read [ROADMAP.MD](./ROADMAP.MD).