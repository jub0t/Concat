<p align="center">
  <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark-dark.png" alt="Concat" width="100" />
</p>

<h1 align="center">Concat</h1>
<h3 align="center">免费、开源、跨平台的视频编辑器。</h3>

<div align="center">

<img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/editor.png" alt="Concat editor" width="100%" />

<p align="center">
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/github/downloads/jub0t/concat/total?style=flat&logo=github&logoColor=F8F8F8&label=Downloads&labelColor=212123&color=0568FD" alt="Total Downloads" /></a>
  <a href="https://github.com/jub0t/Concat/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/jub0t/Concat/ci.yml?style=flat&logo=githubactions&logoColor=F8F8F8&label=Build&labelColor=000000" alt="Build Status" /></a>
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/badge/Version-0.2.12-0568FD?style=flat&logo=semver&logoColor=F8F8F8&labelColor=212123" alt="Concat Version 0.2.12" /></a>
  <a href="https://discord.gg/DVuPfpXfqP"><img src="https://img.shields.io/badge/Discord-Join%20the%20server-5865F2?style=flat&logo=discord&logoColor=F8F8F8&labelColor=000000" alt="Join Concat Discord" /></a>
  <a href="../LICENSE"><img src="https://img.shields.io/badge/License-AGPL%20v3-0568FD?style=flat&logo=gnu&logoColor=F8F8F8&labelColor=212123" alt="License: AGPL-3.0-or-later" /></a>
</p>

<p align="center">
  <a href="https://www.star-history.com/jub0t/concat"><img src="https://api.star-history.com/badge?repo=jub0t/concat&type=rank" alt="Star History Rank" height="62" /></a>
  <a href="https://concat.standalones.org/#download"><img src="../assets/download_button.svg" alt="Download Concat" width="251" /></a>
  <a href="https://trendshift.io/repositories/191379?utm_source=repository-badge&amp;utm_medium=badge&amp;utm_campaign=badge-repository-191379" target="_blank" rel="noopener noreferrer"><img src="https://trendshift.io/api/badge/repositories/191379" alt="jub0t%2FConcat | Trendshift" width="250" height="55"/></a>
</p>

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇳🇱 Nederlands](README.nl.md) · [🇵🇱 Polski](README.pl.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇦🇿 Azərbaycanca](README.az.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇹🇭 ไทย](README.th.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · **🇨🇳 简体中文** · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md) · [🇧🇩 বাংলা](README.bn.md)

> 译自 2026 年 10 月 11 日的英文 README。以英文版为准，它才是持续更新的版本。

## 付费赞助商

<table width="100%">
  <tr>
    <td align="center">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>面向开发者的按量付费代理<br />使用代码 <code>JUB0T</code> 享 20 % 折扣</sub>
    </td>
  </tr>
</table>

</div>


## 关于

Concat 是一款免费、开源的视频编辑器，支持 macOS、Windows、Linux、Android 和 iOS。它覆盖了大多数剪辑所需的功能：自动字幕、人声分离、文字转语音、背景移除、动画与关键帧、特效与标题、多轨剪辑、4K 导出。而且没有任何套路：无水印、无需账号、无需订阅、不上传。

一切都在本地运行，基于带 GPU 合成器的原生 Rust 引擎。安装、拖入素材、开剪。字幕、语音和抠像所用的 AI 模型只需在设置中下载一次，之后离线可用。您的素材永远不会离开您的硬盘。

**适合：** TikTok、Reels 和 Shorts，YouTube 视频，教程与屏幕录制，播客片段，表情包。

**也适合机器：** 提供 JSON-RPC API，脚本和 AI 代理同样可以用它剪视频。

## 亮点

- 🚫 **无水印。无账号。无付费墙。** 永远如此。
- 🔒 **100% 本地。** 不上传任何内容，AI 也不例外。离线可用。
- 💬 **自动字幕。** 用你说的语言，在你的电脑上生成，带样式，直接放上时间线。
- 🎤 **人声分离。** 把人声从音乐或噪音里提出来，或者把人声拿掉，各部分有各自的音量。
- 🗣️ **文字转语音 + 声音克隆。** 免费的本地声音，或用几秒录音克隆任意声音。桌面端可用。
- 🧍 **背景移除、色度抠像和蒙版。** 人物和物体替你抠出，绿幕自动抠像，每个片段上还能叠加多个蒙版，每个蒙版都有自己的模式和关键帧。
- 🌀 **入场、出场和循环动画。** 适用于片段、图片、标题和形状。每一个在选择之前都会在它的卡片上播放。
- ✨ **100 多种特效、滤镜和转场，25 种音效。** 电影感调色、抖动与冲击、复古、擦除，全都在预览中实时呈现。转场就是一个片段，可以像其他片段一样移动和修剪。
- 🎞️ **关键帧。** 位置、缩放、尺寸、旋转、不透明度、音量、特效参数。内置曲线编辑器。
- 🏃 **变速斜坡与曲线。** 恒定或手绘，平滑或直线。定格帧和倒放。
- 🪄 **增强与防抖。** 修复模糊或老旧的素材，最高 2K。让抖动的镜头稳下来。
- ✂️ **剪得快，剪得准。** 分割、修剪、波纹、合并、联动的字幕和叠加层，需要时还有磁性时间线。每一刀都落在帧上，预览和导出完全一致。
- 🎚️ **多轨道、多时间线。** 视频、静态图片、标题和形状，想叠多少轨道就叠多少。一个项目里放多个剪辑版本。混合模式、裁剪、翻转。
- 📝 **标题与形状。** 字体、描边、阴影、底板、彩色表情符号，以及阿拉伯语、中文、印地语、泰语以及另外八种语言的预设。形状支持渐变、星形、轮廓和阴影。
- 🎙️ **旁白与一键人声净化。** 在你的剪辑上直接录音。降噪、人声增强、响度均衡。还有花栗鼠、机器人、电话等音效。
- 📤 **导出。** H.264、HEVC、AV1，使用你电脑自带的编码器。最高 4K 60，10 位色彩。
- ⚡ **快。** 内置代理，4K 拖动也顺滑。标题在每个核心上绘制，窗口一打开，所有特效就已就绪。
- 🛟 **任何机器上都能继续运行。** 显卡芯片罢工时，预览依然会继续绘制。
- 🤖 **可脚本化。** 提供 JSON-RPC API，脚本和 AI 代理可以用它剪视频。从源码构建还会加上 gRPC 和命令行工具。
- 🖥️ **macOS、Windows、Linux、Android、iOS。** 26 种语言。同一个应用，同样的项目文件。

## 下载

两个入口：

1. **[官网](https://concat.standalones.org/#download)** 会给您适合您机器的版本。从这里开始。
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** 有每个平台的每个版本，附带安装程序、软件包和校验和。想自己挑选时用它。

Concat 处于 **测试版**：能用，但还有毛边。遇到了请 [告诉我们](https://github.com/jub0t/Concat/issues)。

**平台**

- ✅ **Windows** · x86_64（ARM 版本已暂停，见 [#326](https://github.com/jub0t/Concat/issues/326)）。安装程序和 `.msi`。如果 SmartScreen 拦截未签名版本：**更多信息** › **仍要运行**
- ✅ **macOS** · Intel 和 Apple 芯片。如果 macOS 拒绝打开未签名版本：`xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 和 ARM。`.deb`、`.rpm`、`.AppImage` 和 Arch 软件包。另有 **[Flatpark](https://flatpark.org/apps/app.concat.editor/)** 上的 Flatpak，这是一个社区 Flatpak 仓库，封装每个版本的 x86_64 `.deb` 并随之更新
- ✅ **Android** · 手机和平板
- ✅ **iOS / iPadOS** · iPhone 和 iPad，需侧载

✅ 已支持 · 🚧 开发中 · 🧪 待测试

**系统要求** 和可选模型的大小见 [官网](https://concat.standalones.org/guides/system-requirements)。

**[Code signing policy](../CODE_SIGNING.md)**

## 如何贡献

> [!IMPORTANT]
> 最好的贡献方式是从 [Releases](https://github.com/jub0t/Concat/releases) 页面拿一个版本来用：找到它哪里会坏，说出它哪里可以更好。
>
> 准备写代码了？[CONTRIBUTING.md](../CONTRIBUTING.md) 介绍了环境搭建、目录结构、要运行的检查以及贡献的许可方式。要从脚本、服务或代理驱动 Concat？[开发者文档](https://concat.standalones.org/docs) 介绍了 Concat API 及其传输方式：JSON-RPC 和 gRPC。项目在 [这个 Discussion](https://github.com/jub0t/Concat/discussions/3) 里发布。
> 
> 贡献者可以在 Discord 服务器申请 `@Contributor` 身份组，开口要就行。

## Star 历史

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
</a>

## 贡献者

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## DMCA

如果您认为本仓库、其发布版本，或其分发的某个软件包或模型中的内容侵犯了您的版权，请发送邮件至 [jub0trd@gmail.com](mailto:jub0trd@gmail.com)，并附上：对您拥有的作品的描述；侵权内容在此处出现位置的链接；您的联系方式；一份声明，表明您善意地相信该使用未获得您、您的代理人或法律的授权；以及一份在承担伪证罪责任的前提下作出的声明，表明通知内容准确无误，且您是权利人或经授权代表权利人行事，并附有您的手写签名或电子签名。有效的通知会得到及时处理；任何内容被移除的人，都可以向同一地址发送反通知。
