<div align="center">
<table width="100%">
  <tr>
    <td align="left" width="120">
      <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark.png" alt="Concat" width="100" />
    </td>
    <td align="">
      <h1>Concat</h1>
      <h3 style="margin-top: -10px;">Der wirklich kostenlose, quelloffene CapCut-Ersatz für alle Plattformen.</h3>
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

[🇬🇧 English](../README.md) · **🇩🇪 Deutsch** · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md)

> Übersetzt aus der englischen README vom 2. Oktober 2026. Die englische Fassung ist die, die aktuell gehalten wird.

## Zahlende Sponsoren

<table width="100%">
  <tr>
    <td align="center" width="50%">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>Pay-as-you-go-Proxys für Entwickler<br />Mit dem Code <code>JUB0T</code> 20 % Rabatt</sub>
    </td>
    <td align="center" width="50%">
      <a href="#sponsoring"><b>Dein Logo hier</b></a><br />
      <sub><a href="#sponsoring">Sponsere Concat</a> und dein Name, dein Logo und dein Link stehen an dieser Stelle</sub>
    </td>
  </tr>
</table>

</div>


## Über Concat

Concat ist ein kostenloser, quelloffener Videoeditor und eine CapCut-Alternative für macOS, Windows, Linux und Android. Er kann das, wofür man CapCut tatsächlich öffnet: automatische Untertitel, Text-zu-Sprache, Hintergrundentfernung, Keyframe-Animation, Effekte und Titel, Schnitt auf mehreren Spuren, 4K-Export. Ohne die Haken: kein Wasserzeichen, kein Konto, kein Abo, kein Upload.

Alles läuft lokal auf einer nativen Rust-Engine mit GPU-Compositor. Installieren, Material hineinziehen, schneiden. Die KI-Modelle für Untertitel, Stimmen und Freisteller werden einmal in den Einstellungen geladen und arbeiten danach offline. Dein Material verlässt deine Festplatte nie.

**Gut für:** TikTok, Reels und Shorts, YouTube-Videos, Tutorials und Bildschirmaufnahmen, Podcast-Clips, Memes.

**Auch für Maschinen:** eine JSON-RPC-, gRPC- und MCP-API, damit auch Skripte und KI-Agenten damit Video schneiden können.

## Highlights

- 🚫 **Keine Wasserzeichen. Kein Konto. Keine Bezahlschranke.** Niemals.
- 🔒 **100 % lokal.** Nichts wird hochgeladen. Funktioniert offline.
- 💬 **Automatische Untertitel.** Lokales Whisper. Modellgröße wählen, gestylte Untertitel auf der Zeitleiste bekommen.
- 🗣️ **Text-zu-Sprache + Stimmklonen.** Kostenlose lokale Stimmen, oder jede Stimme aus ein paar Sekunden Aufnahme.
- 🧍 **Hintergrundentfernung.** Personen, Objekte, oder die Maske selbst malen.
- 🎞️ **Keyframes.** Position, Skalierung, Drehung, Deckkraft, Lautstärke, Effektparameter. Kurveneditor eingebaut.
- ✨ **Über 170 Effekte, Filter, Übergänge und Textanimationen.** GPU-gerendert, live in der Vorschau.
- ✂️ **Schnell schneiden.** Teilen, trimmen, Ripple, zusammenführen, Standbild, Geschwindigkeit. Magnetische Zeitleiste, wenn du willst.
- 🎚️ **Mehrere Spuren, mehrere Zeitleisten.** Mehrere Schnitte in einem Projekt. Mischmodi, Zuschnitt, Spiegelungen.
- 📝 **Titel.** Schriften, Kontur, Schatten, Hintergrundplatte. Vorlagen zum Loslegen.
- 🎙️ **Stimmbereinigung mit einem Schalter.** Entrauschen, Stimme verbessern, Lautheit angleichen. Dazu Chipmunk, Roboter, Telefon und Freunde.
- 📤 **Export.** H.264, HEVC, AV1. Bis 4K 60, 10-Bit-Farbe.
- 🦀 **Native Rust-Engine.** GPU-Compositor, Proxys, Hardware-Decoding. 4K scrubbt flüssig.
- 🤖 **Skriptbar.** JSON-RPC-, gRPC- und MCP-API plus CLI. KI-Agenten können damit Video schneiden.
- 🖥️ **macOS, Windows, Linux, Android.** 14 Sprachen. Dieselbe App, dieselben Projektdateien.

## Download

Zwei Wege hinein:

1. **[Die Website](https://concatenate.pages.dev/#download)** gibt dir den passenden Build für deinen Rechner. Fang hier an.
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** hat jeden Build für jede Plattform, mit Installern, Paketen und Prüfsummen. Für den Fall, dass du selbst wählen willst.

Concat ist in der **Beta**: Es funktioniert, hat aber noch Kanten. [Sag Bescheid](https://github.com/jub0t/Concat/issues), wenn du eine findest.

**Plattformen**

- ✅ **Windows** · x86_64 und ARM. Ein Setup und eine `.msi`. Wenn SmartScreen einen unsignierten Build stoppt: **Weitere Informationen** › **Trotzdem ausführen**
- ✅ **macOS** · Intel und Apple Silicon. Wenn macOS einen unsignierten Build nicht öffnen will: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 und ARM. `.deb`, `.rpm`, `.AppImage` und ein Arch-Paket. Außerdem ein Flatpak auf **[Flatpark](https://flatpark.org/apps/app.concat.editor/)**, einem Community-Flatpak-Remote, das die x86_64-`.deb` jeder Version verpackt und mit ihr aktualisiert
- ✅ **Android** · Telefone und Tablets
- ✅ **iOS / iPadOS** · iPhone und iPad, per Sideload

✅ Unterstützt · 🚧 In Arbeit · 🧪 Noch zu testen

**Systemanforderungen** und die Größen der optionalen Modelle stehen auf [der Website](https://concatenate.pages.dev/guides/system-requirements).

## Loslegen

Herunterladen, öffnen, Material hineinziehen, schneiden. Kein Konto, keine Einrichtung.

**Etwas melden:** Jeder Lauf schreibt ein Protokoll, und unter Einstellungen › Über gibt es den Knopf, der es öffnet, neben dem, der deine Systeminformationen kopiert. Häng beides an ein [Issue](https://github.com/jub0t/Concat/issues), und der Bericht kommt mit allem an, was er braucht. Die letzten zehn Läufe werden aufbewahrt, der von gestern ist also noch da; nichts wird je von selbst irgendwohin geschickt.

## Mitmachen

> [!IMPORTANT]
> Am besten hilfst du, indem du dir einen Build von der [Releases](https://github.com/jub0t/Concat/releases)-Seite holst und ihn benutzt: finde, wo er bricht, und sag, wo er besser sein könnte.
>
> Bereit, Code zu schreiben? [CONTRIBUTING.md](../CONTRIBUTING.md) beschreibt die Einrichtung, den Aufbau des Baums, die auszuführenden Prüfungen und wie Beiträge lizenziert werden. Du steuerst Concat aus einem Skript, einem Dienst oder einem Agenten? [Die Entwicklerdokumentation](https://concatenate.pages.dev/docs) beschreibt die Concat-API und ihre Transporte: JSON-RPC, gRPC und MCP. [In dieser Discussion](https://github.com/jub0t/Concat/discussions/3) wurde das Projekt angekündigt.
> 
> Mitwirkende können sich auf dem Discord-Server eine `@Contributor`-Rolle holen, einfach danach fragen.

## Mitwirkende

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## Sternverlauf

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&theme=dark&legend=bottom-right" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
 </picture>
</a>

<a id="sponsoring"></a>
## Sponsoring

Concat hat keine Bezahlschranke und wird nie eine haben: kein Wasserzeichen, kein Konto, keine Bezahlstufe. Sponsoring ist das, was an deren Stelle tritt. Wenn Concat für dich ein Abo ersetzt hat, hält ein Bruchteil davon es am Laufen.

**Wofür es verwendet wird**

Die [Roadmap](https://concatenate.pages.dev/roadmap) zeigt, was Sponsoring bezahlt und was jedes Stück kostet.

Wähle eine Stufe auf [der Website](https://concatenate.pages.dev/#sponsor).

<a href="https://concatenate.pages.dev/#sponsor"><img src="https://img.shields.io/badge/Sponsor-Concat-0568FD?style=flat-square&labelColor=212123" alt="Sponsor Concat on the website" /></a>

Gerade nicht in der Lage, etwas beizusteuern? Ein Stern, ein Fehlerbericht oder ein Wort an jemanden, der Video schneidet, zählt auch eine Menge.
