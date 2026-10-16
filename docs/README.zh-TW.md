<p align="center">
  <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark-dark.png" alt="Concat" width="100" />
</p>

<h1 align="center">Concat</h1>
<h3 align="center">免費、開源、跨平台的 CapCut 替代方案。</h3>

<div align="center">

<img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/editor.png" alt="Concat editor" width="100%" />

<p align="center">
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/github/downloads/jub0t/concat/total?style=flat&logo=github&logoColor=F8F8F8&label=Downloads&labelColor=212123&color=0568FD" alt="Total Downloads" /></a>
  <a href="https://github.com/jub0t/Concat/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/jub0t/Concat/ci.yml?style=flat&logo=githubactions&logoColor=F8F8F8&label=Build&labelColor=000000" alt="Build Status" /></a>
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/badge/Version-0.2.7-0568FD?style=flat&logo=semver&logoColor=F8F8F8&labelColor=212123" alt="Concat Version 0.2.7" /></a>
  <a href="https://discord.gg/DVuPfpXfqP"><img src="https://img.shields.io/badge/Discord-Join%20the%20server-5865F2?style=flat&logo=discord&logoColor=F8F8F8&labelColor=000000" alt="Join Concat Discord" /></a>
  <a href="../LICENSE"><img src="https://img.shields.io/badge/License-AGPL%20v3-0568FD?style=flat&logo=gnu&logoColor=F8F8F8&labelColor=212123" alt="License: AGPL-3.0-or-later" /></a>
</p>

<p align="center">
  <a href="https://www.star-history.com/jub0t/concat"><img src="https://api.star-history.com/badge?repo=jub0t/concat&type=rank" alt="Star History Rank" height="62" /></a>
  <a href="https://concatenate.pages.dev/#download"><img src="../assets/download_button.svg" alt="Download Concat" width="251" /></a>
</p>

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇳🇱 Nederlands](README.nl.md) · [🇵🇱 Polski](README.pl.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇹🇭 ไทย](README.th.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · **🇹🇼 繁體中文** · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md) · [🇧🇩 বাংলা](README.bn.md)

> 譯自 2026 年 10 月 11 日的英文 README。以英文版為準，它才是持續更新的版本。

## 付費贊助商

<table width="100%">
  <tr>
    <td align="center">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>給開發者的按量計費代理<br />使用代碼 <code>JUB0T</code> 享 20 % 折扣</sub>
    </td>
  </tr>
</table>

</div>


## 關於

Concat 是一款免費、開源的影片編輯器，是 macOS、Windows、Linux 與 Android 上的 CapCut 替代方案。它涵蓋了人們真正打開 CapCut 要做的事：自動字幕、文字轉語音、背景移除、關鍵影格動畫、特效與標題、多軌剪輯、4K 輸出。而且沒有任何陷阱：無浮水印、不需帳號、不需訂閱、不上傳。

一切都在本機執行，基於帶 GPU 合成器的原生 Rust 引擎。安裝、拖入素材、開剪。字幕、語音與去背所用的 AI 模型只需在設定中下載一次，之後離線可用。您的素材永遠不會離開您的硬碟。

**適合：** TikTok、Reels 與 Shorts，YouTube 影片，教學與螢幕錄影，Podcast 片段，迷因。

**也適合機器：** 提供 JSON-RPC API，指令碼與 AI 代理同樣可以用它剪影片。

## 亮點

- 🚫 **無浮水印。無帳號。無付費牆。** 永遠如此。
- 🔒 **100 % 本機。** 不上傳任何內容。離線可用。
- 💬 **自動字幕。** 本機 Whisper。選一個模型大小，帶樣式的字幕就排進時間軸。
- 🗣️ **文字轉語音 + 聲音複製。** 免費的本機聲音，或用幾秒鐘錄音複製任何聲音。
- 🧍 **背景移除。** 人物、物體，或者自己塗抹遮罩。
- 🎞️ **關鍵影格。** 位置、縮放、旋轉、不透明度、音量、特效參數。內建曲線編輯器。
- ✨ **100 種特效、濾鏡與轉場。** GPU 算繪，預覽中即時呈現。
- ✂️ **快剪。** 分割、修剪、漣漪、合併、定格、變速。想要的話還有磁性時間軸。
- 🎚️ **多軌、多時間軸。** 一個專案裡放多個剪輯版本。混合模式、裁切、翻轉。
- 📝 **標題。** 字型、描邊、陰影、背景底板。有預設可作起點。
- 🎙️ **一鍵人聲淨化。** 降噪、增強人聲、響度均衡。還有花栗鼠、機器人、電話等效果。
- 📤 **輸出。** H.264、HEVC、AV1。最高 4K 60，10 位元色彩。
- 🦀 **原生 Rust 引擎。** GPU 合成器、代理、硬體解碼。4K 拖曳也順暢。
- 🤖 **可指令碼化。** 提供 JSON-RPC API，指令碼與 AI 代理可以用它剪影片。從原始碼建置還會加上 gRPC 與命令列工具。
- 🖥️ **macOS、Windows、Linux、Android。** 25 種語言。同一個應用程式，同樣的專案檔。

## 下載

兩個入口：

1. **[官網](https://concatenate.pages.dev/#download)** 會給您適合您機器的版本。從這裡開始。
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** 有每個平台的每個版本，附帶安裝程式、套件與校驗和。想自己挑選時用它。

Concat 處於 **測試版**：能用，但還有毛邊。遇到了請 [告訴我們](https://github.com/jub0t/Concat/issues)。

**平台**

- ✅ **Windows** · x86_64（ARM 版本已暫停，見 [#326](https://github.com/jub0t/Concat/issues/326)）。安裝程式與 `.msi`。如果 SmartScreen 攔截未簽署版本：**其他資訊** › **仍要執行**
- ✅ **macOS** · Intel 與 Apple 晶片。如果 macOS 拒絕開啟未簽署版本：`xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 與 ARM。`.deb`、`.rpm`、`.AppImage` 與 Arch 套件。另有 **[Flatpark](https://flatpark.org/apps/app.concat.editor/)** 上的 Flatpak，這是一個社群 Flatpak 儲存庫，封裝每個版本的 x86_64 `.deb` 並隨之更新
- ✅ **Android** · 手機與平板
- ✅ **iOS / iPadOS** · iPhone 與 iPad，需側載

✅ 已支援 · 🚧 開發中 · 🧪 待測試

**系統需求** 與可選模型的大小見 [官網](https://concatenate.pages.dev/guides/system-requirements)。

**[Code signing policy](../CODE_SIGNING.md)**

## 如何貢獻

> [!IMPORTANT]
> 最好的貢獻方式是從 [Releases](https://github.com/jub0t/Concat/releases) 頁面拿一個版本來用：找到它哪裡會壞，說出它哪裡可以更好。
>
> 準備寫程式了？[CONTRIBUTING.md](../CONTRIBUTING.md) 介紹了環境建置、目錄結構、要執行的檢查以及貢獻的授權方式。要從指令碼、服務或代理驅動 Concat？[開發者文件](https://concatenate.pages.dev/docs) 介紹了 Concat API 及其傳輸方式：JSON-RPC 與 gRPC。專案在 [這個 Discussion](https://github.com/jub0t/Concat/discussions/3) 中發布。
> 
> 貢獻者可以在 Discord 伺服器申請 `@Contributor` 身分組，開口要就行。

## Star 歷史

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
</a>

## 貢獻者

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## DMCA

如果您認為本儲存庫、其發行版本，或其散布的某個套件或模型中的內容侵害了您的著作權，請寄信至 [jub0trd@gmail.com](mailto:jub0trd@gmail.com)，並附上：您所擁有之著作的說明；侵權內容在此處出現位置的連結；您的聯絡資料；一份聲明，表示您善意相信該使用未經您、您的代理人或法律授權；以及一份在願負偽證罪責任下所作的聲明，表示通知內容正確無誤，且您是權利人或經授權代表權利人行事，並附有您的親筆簽名或電子簽名。有效的通知會迅速處理；任何內容遭移除的人，都可以向同一地址寄送反通知。
