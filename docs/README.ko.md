<div align="center">
<table width="100%">
  <tr>
    <td align="left" width="120">
      <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark.png" alt="Concat" width="100" />
    </td>
    <td align="">
      <h1>Concat</h1>
      <h3 style="margin-top: -10px;">진정으로 무료이며 오픈 소스인 크로스 플랫폼 CapCut 대체재.</h3>
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

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇯🇵 日本語](README.ja.md) · **🇰🇷 한국어** · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md)

> 2026년 10월 2일 기준 영어 README를 번역한 것입니다. 최신 내용은 영어판이 유지합니다.

## 유료 스폰서

<table width="100%">
  <tr>
    <td align="center" width="50%">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>개발자를 위한 종량제 프록시<br />코드 <code>JUB0T</code>로 20 % 할인</sub>
    </td>
    <td align="center" width="50%">
      <a href="#sponsoring"><b>여기에 당신의 로고를</b></a><br />
      <sub><a href="#sponsoring">Concat을 후원하면</a> 당신의 이름, 로고, 링크가 이 자리에 실립니다</sub>
    </td>
  </tr>
</table>

</div>


## 소개

Concat은 macOS, Windows, Linux, Android용 무료 오픈 소스 비디오 편집기이자 CapCut의 대안입니다. 사람들이 실제로 CapCut을 여는 이유를 모두 다룹니다: 자동 자막, 텍스트 음성 변환, 배경 제거, 키프레임 애니메이션, 효과와 타이틀, 멀티트랙 편집, 4K 내보내기. 단, 함정은 없습니다: 워터마크 없음, 계정 없음, 구독 없음, 업로드 없음.

모든 것이 GPU 컴포지터를 갖춘 네이티브 Rust 엔진 위에서 로컬로 실행됩니다. 설치하고, 영상을 끌어다 놓고, 자르면 됩니다. 자막, 음성, 배경 제거용 AI 모델은 설정에서 한 번만 내려받으면 그 뒤로는 오프라인으로 동작합니다. 당신의 영상은 디스크 밖으로 나가지 않습니다.

**이런 데 좋습니다:** TikTok, Reels, Shorts, YouTube 영상, 튜토리얼과 화면 녹화, 팟캐스트 클립, 밈.

**기계를 위해서도:** JSON-RPC, gRPC, MCP API가 있어 스크립트와 AI 에이전트도 이것으로 영상을 편집할 수 있습니다.

## 주요 기능

- 🚫 **워터마크 없음. 계정 없음. 유료 장벽 없음.** 영원히.
- 🔒 **100 % 로컬.** 아무것도 업로드하지 않습니다. 오프라인에서 동작.
- 💬 **자동 자막.** 로컬 Whisper. 모델 크기를 고르면 스타일이 입혀진 자막이 타임라인에 놓입니다.
- 🗣️ **텍스트 음성 변환 + 음성 복제.** 무료 로컬 음성, 또는 몇 초의 녹음으로 어떤 목소리든.
- 🧍 **배경 제거.** 사람, 사물, 또는 직접 마스크를 칠하세요.
- 🎞️ **키프레임.** 위치, 크기, 회전, 불투명도, 볼륨, 효과 매개변수. 커브 편집기 내장.
- ✨ **170개 이상의 효과, 필터, 전환, 텍스트 애니메이션.** GPU 렌더링, 미리보기에서 실시간.
- ✂️ **빠른 편집.** 분할, 트림, 리플, 병합, 프레임 고정, 속도. 원하면 자석 타임라인.
- 🎚️ **멀티트랙, 멀티타임라인.** 한 프로젝트에 여러 편집본. 혼합 모드, 자르기, 뒤집기.
- 📝 **타이틀.** 글꼴, 외곽선, 그림자, 배경 판. 시작점이 되는 프리셋.
- 🎙️ **스위치 하나로 음성 정리.** 잡음 제거, 음성 강조, 음량 평준화. 그리고 다람쥐, 로봇, 전화 목소리 등.
- 📤 **내보내기.** H.264, HEVC, AV1. 최대 4K 60, 10비트 색상.
- 🦀 **네이티브 Rust 엔진.** GPU 컴포지터, 프록시, 하드웨어 디코딩. 4K도 부드럽게 스크러빙.
- 🤖 **스크립트 가능.** JSON-RPC, gRPC, MCP API와 CLI. AI 에이전트가 이것으로 영상을 편집할 수 있습니다.
- 🖥️ **macOS, Windows, Linux, Android.** 14개 언어. 같은 앱, 같은 프로젝트 파일.

## 다운로드

두 가지 길:

1. **[웹사이트](https://concatenate.pages.dev/#download)** 가 당신의 기기에 맞는 빌드를 건네줍니다. 여기서 시작하세요.
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** 에는 모든 플랫폼의 모든 빌드가 설치 프로그램, 패키지, 체크섬과 함께 있습니다. 직접 고르고 싶을 때.

Concat은 **베타**입니다: 동작하지만 아직 거친 부분이 있습니다. 발견하면 [알려주세요](https://github.com/jub0t/Concat/issues).

**플랫폼**

- ✅ **Windows** · x86_64와 ARM. 설치 프로그램과 `.msi`. SmartScreen이 서명되지 않은 빌드를 막으면: **추가 정보** › **실행**
- ✅ **macOS** · Intel과 Apple Silicon. macOS가 서명되지 않은 빌드를 열지 않으면: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64와 ARM. `.deb`, `.rpm`, `.AppImage`, Arch 패키지. 그리고 **[Flatpark](https://flatpark.org/apps/app.concat.editor/)**의 Flatpak도 있습니다. 각 릴리스의 x86_64 `.deb`를 감싸고 릴리스와 함께 업데이트되는 커뮤니티 Flatpak 원격 저장소입니다
- ✅ **Android** · 휴대폰과 태블릿
- ✅ **iOS / iPadOS** · iPhone과 iPad, 사이드로드

✅ 지원 · 🚧 작업 중 · 🧪 테스트 예정

**시스템 요구 사항**과 선택 모델의 크기는 [웹사이트](https://concatenate.pages.dev/guides/system-requirements)에 있습니다.

## 시작하기

내려받고, 열고, 영상을 끌어다 놓고, 자르세요. 계정도 설정도 필요 없습니다.

**문제를 알리려면:** 실행할 때마다 로그가 기록되며, 설정 › 정보에는 로그를 여는 버튼과 시스템 정보를 복사하는 버튼이 있습니다. 둘 다 [issue](https://github.com/jub0t/Concat/issues)에 첨부하면 필요한 모든 것이 담긴 보고가 됩니다. 최근 열 번의 실행이 보관되므로 어제 것도 남아 있습니다. 아무것도 저절로 어디론가 전송되지 않습니다.

## 기여하는 법

> [!IMPORTANT]
> 가장 좋은 기여는 [Releases](https://github.com/jub0t/Concat/releases) 페이지에서 빌드를 받아 써 보는 것입니다: 어디서 깨지는지 찾고, 어디가 더 나아질 수 있는지 말해 주세요.
>
> 코드를 쓸 준비가 되셨나요? [CONTRIBUTING.md](../CONTRIBUTING.md)에 설정, 트리 구조, 실행할 검사, 기여의 라이선스가 적혀 있습니다. 스크립트, 서비스, 에이전트에서 Concat을 다루시나요? [개발자 문서](https://concatenate.pages.dev/docs)에 Concat API와 전송 방식(JSON-RPC, gRPC, MCP)이 있습니다. 프로젝트는 [이 Discussion](https://github.com/jub0t/Concat/discussions/3)에서 공개되었습니다.
> 
> 기여자는 Discord 서버에서 `@Contributor` 역할을 받을 수 있습니다. 요청만 하세요.

## 기여자

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## 스타 추이

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&theme=dark&legend=bottom-right" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
 </picture>
</a>

<a id="sponsoring"></a>
## 후원

Concat에는 유료 장벽이 없고 앞으로도 없을 것입니다: 워터마크 없음, 계정 없음, 유료 등급 없음. 그 자리를 대신하는 것이 후원입니다. Concat이 당신의 구독을 대신하게 되었다면, 그 일부가 Concat을 계속 굴러가게 합니다.

**쓰임새**

[로드맵](https://concatenate.pages.dev/roadmap)에 후원금이 무엇에 쓰이고 각 항목에 얼마가 드는지 나와 있습니다.

[웹사이트](https://concatenate.pages.dev/#sponsor)에서 등급을 고르세요.

<a href="https://concatenate.pages.dev/#sponsor"><img src="https://img.shields.io/badge/Sponsor-Concat-0568FD?style=flat-square&labelColor=212123" alt="Sponsor Concat on the website" /></a>

지금은 후원이 어렵더라도 스타 하나, 버그 보고 하나, 영상을 편집하는 누군가에게 건네는 한마디도 큰 힘이 됩니다.
