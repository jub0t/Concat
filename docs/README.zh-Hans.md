<div align="center">
<table width="100%">
  <tr>
    <td align="left" width="120">
      <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark.png" alt="Concat" width="100" />
    </td>
    <td align="">
      <h1>Concat</h1>
      <h3 style="margin-top: -10px;">真正免费、开源、跨平台的 CapCut 替代品。</h3>
    </td>
  </tr>
</table>

<img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/editor.png" alt="Concat editor" width="100%" />

<p align="center">
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/github/downloads/jub0t/concat/total?style=flat-square&logo=github&logoColor=F8F8F8&label=Downloads&labelColor=212123&color=0568FD" alt="Total Downloads" /></a>
  <a href="https://github.com/jub0t/Concat/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/jub0t/Concat/ci.yml?style=flat&logo=githubactions&logoColor=F8F8F8&label=Build&labelColor=000000" alt="Build Status" /></a>
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/badge/Version-0.2.6-0568FD?style=flat-square&logo=semver&logoColor=F8F8F8&labelColor=212123" alt="Concat Version 0.2.5" /></a>
  <a href="https://discord.gg/DVuPfpXfqP"><img src="https://img.shields.io/badge/Discord-Join%20the%20server-5865F2?style=flat&logo=discord&logoColor=F8F8F8&labelColor=000000" alt="Join Concat Discord" /></a>
  <a href="../LICENSE"><img src="https://img.shields.io/badge/License-AGPL%20v3-0568FD?style=flat-square&logo=gnu&logoColor=F8F8F8&labelColor=212123" alt="License: AGPL-3.0-or-later" /></a>
  <a href="https://concatenate.pages.dev/#sponsor"><img src="https://img.shields.io/badge/Sponsor-Concat-0568FD?style=flat-square&labelColor=212123" alt="Sponsor Concat" /></a>
</p>

<p align="center">
  <a href="https://concatenate.pages.dev/#download"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/download_button.svg" alt="Download Concat" width="220" /></a>
</p>

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · **🇨🇳 简体中文** · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md)

> 译自 2026 年 10 月 2 日的英文 README。以英文版为准，它才是持续更新的版本。

## 付费赞助商

<table width="100%">
  <tr>
    <td align="center" width="50%">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>面向开发者的按量付费代理<br />使用代码 <code>JUB0T</code> 享 20 % 折扣</sub>
    </td>
    <td align="center" width="50%">
      <a href="#sponsoring"><b>您的 Logo 放这里</b></a><br />
      <sub><a href="#sponsoring">赞助 Concat</a> 您的名字、Logo 和链接就会出现在这个位置</sub>
    </td>
  </tr>
</table>

</div>


## 关于

Concat 是一款免费、开源的视频编辑器，是 macOS、Windows、Linux 和 Android 上的 CapCut 替代品。它覆盖了人们真正打开 CapCut 要做的事：自动字幕、文字转语音、背景移除、关键帧动画、特效与标题、多轨剪辑、4K 导出。而且没有任何套路：无水印、无需账号、无需订阅、不上传。

一切都在本地运行，基于带 GPU 合成器的原生 Rust 引擎。安装、拖入素材、开剪。字幕、语音和抠像所用的 AI 模型只需在设置中下载一次，之后离线可用。您的素材永远不会离开您的硬盘。

**适合：** TikTok、Reels 和 Shorts，YouTube 视频，教程与屏幕录制，播客片段，表情包。

**也适合机器：** 提供 JSON-RPC、gRPC 和 MCP API，脚本和 AI 代理同样可以用它剪视频。

## 亮点

- 🚫 **无水印。无账号。无付费墙。** 永远如此。
- 🔒 **100 % 本地。** 不上传任何内容。离线可用。
- 💬 **自动字幕。** 本地 Whisper。选一个模型大小，带样式的字幕就排进时间线。
- 🗣️ **文字转语音 + 声音克隆。** 免费的本地声音，或用几秒钟录音克隆任何声音。
- 🧍 **背景移除。** 人物、物体，或者自己涂抹遮罩。
- 🎞️ **关键帧。** 位置、缩放、旋转、不透明度、音量、特效参数。内置曲线编辑器。
- ✨ **170 多种特效、滤镜、转场和文字动画。** GPU 渲染，预览中实时呈现。
- ✂️ **快剪。** 分割、修剪、涟漪、合并、定格、变速。想要的话还有磁性时间线。
- 🎚️ **多轨道、多时间线。** 一个项目里放多个剪辑版本。混合模式、裁剪、翻转。
- 📝 **标题。** 字体、描边、阴影、背景底板。有预设可作起点。
- 🎙️ **一键人声净化。** 降噪、增强人声、响度均衡。还有花栗鼠、机器人、电话等效果。
- 📤 **导出。** H.264、HEVC、AV1。最高 4K 60，10 位色彩。
- 🦀 **原生 Rust 引擎。** GPU 合成器、代理、硬件解码。4K 拖动也顺滑。
- 🤖 **可脚本化。** JSON-RPC、gRPC 和 MCP API，外加 CLI。AI 代理可以用它剪视频。
- 🖥️ **macOS、Windows、Linux、Android。** 14 种语言。同一个应用，同样的项目文件。

## 下载

两个入口：

1. **[官网](https://concatenate.pages.dev/#download)** 会给您适合您机器的版本。从这里开始。
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** 有每个平台的每个版本，附带安装程序、软件包和校验和。想自己挑选时用它。

Concat 处于 **测试版**：能用，但还有毛边。遇到了请 [告诉我们](https://github.com/jub0t/Concat/issues)。

**平台**

- ✅ **Windows** · x86_64 和 ARM。安装程序和 `.msi`。如果 SmartScreen 拦截未签名版本：**更多信息** › **仍要运行**
- ✅ **macOS** · Intel 和 Apple 芯片。如果 macOS 拒绝打开未签名版本：`xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 和 ARM。`.deb`、`.rpm`、`.AppImage` 和 Arch 软件包。另有 **[Flatpark](https://flatpark.org/apps/app.concat.editor/)** 上的 Flatpak，这是一个社区 Flatpak 仓库，封装每个版本的 x86_64 `.deb` 并随之更新
- ✅ **Android** · 手机和平板
- ✅ **iOS / iPadOS** · iPhone 和 iPad，需侧载

✅ 已支持 · 🚧 开发中 · 🧪 待测试

**系统要求** 和可选模型的大小见 [官网](https://concatenate.pages.dev/guides/system-requirements)。

## 开始使用

下载、打开、拖入素材、开剪。无需账号，无需设置。

**反馈问题：** 每次运行都会写日志，设置 › 关于 里有打开日志的按钮，旁边是复制系统信息的按钮。把两者附到一个 [issue](https://github.com/jub0t/Concat/issues) 里，报告就带齐了所需的一切。保留最近十次运行，所以昨天的还在；任何内容都不会自动发送到任何地方。

## 如何贡献

> [!IMPORTANT]
> 最好的贡献方式是从 [Releases](https://github.com/jub0t/Concat/releases) 页面拿一个版本来用：找到它哪里会坏，说出它哪里可以更好。
>
> 准备写代码了？[CONTRIBUTING.md](../CONTRIBUTING.md) 介绍了环境搭建、目录结构、要运行的检查以及贡献的许可方式。要从脚本、服务或代理驱动 Concat？[开发者文档](https://concatenate.pages.dev/docs) 介绍了 Concat API 及其传输方式：JSON-RPC、gRPC 和 MCP。项目在 [这个 Discussion](https://github.com/jub0t/Concat/discussions/3) 里发布。
> 
> 贡献者可以在 Discord 服务器申请 `@Contributor` 身份组，开口要就行。

## 贡献者

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## Star 历史

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&theme=dark&legend=bottom-right" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
 </picture>
</a>

<a id="sponsoring"></a>
## 赞助

Concat 没有付费墙，以后也不会有：无水印、无账号、无付费档。赞助就是替代它的方式。如果 Concat 替您省下了一份订阅，拿出其中一小部分就能让它继续走下去。

**钱用在哪**

[路线图](https://concatenate.pages.dev/roadmap) 列出了赞助用于哪些事，以及每一项的花费。

在 [官网](https://concatenate.pages.dev/#sponsor) 选一个档位。

<a href="https://concatenate.pages.dev/#sponsor"><img src="https://img.shields.io/badge/Sponsor-Concat-0568FD?style=flat-square&labelColor=212123" alt="Sponsor Concat on the website" /></a>

暂时不方便出力？一个 Star、一份 bug 报告，或者向剪视频的朋友提一句，同样意义重大。
