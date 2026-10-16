<p align="center">
  <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark-dark.png" alt="Concat" width="100" />
</p>

<h1 align="center">Concat</h1>
<h3 align="center">Der kostenlose, quelloffene CapCut-Ersatz für alle Plattformen.</h3>

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

[🇬🇧 English](../README.md) · **🇩🇪 Deutsch** · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇳🇱 Nederlands](README.nl.md) · [🇵🇱 Polski](README.pl.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇹🇭 ไทย](README.th.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md) · [🇧🇩 বাংলা](README.bn.md)

> Übersetzt aus der englischen README vom 11. Oktober 2026. Die englische Fassung ist die, die aktuell gehalten wird.

## Zahlende Sponsoren

<table width="100%">
  <tr>
    <td align="center">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>Pay-as-you-go-Proxys für Entwickler<br />Mit dem Code <code>JUB0T</code> 20 % Rabatt</sub>
    </td>
  </tr>
</table>

</div>


## Über Concat

Concat ist ein kostenloser, quelloffener Videoeditor und eine CapCut-Alternative für macOS, Windows, Linux und Android. Er kann das, wofür man CapCut tatsächlich öffnet: automatische Untertitel, Text-zu-Sprache, Hintergrundentfernung, Keyframe-Animation, Effekte und Titel, Schnitt auf mehreren Spuren, 4K-Export. Ohne die Haken: kein Wasserzeichen, kein Konto, kein Abo, kein Upload.

Alles läuft lokal auf einer nativen Rust-Engine mit GPU-Compositor. Installieren, Material hineinziehen, schneiden. Die KI-Modelle für Untertitel, Stimmen und Freisteller werden einmal in den Einstellungen geladen und arbeiten danach offline. Dein Material verlässt deine Festplatte nie.

**Gut für:** TikTok, Reels und Shorts, YouTube-Videos, Tutorials und Bildschirmaufnahmen, Podcast-Clips, Memes.

**Auch für Maschinen:** eine JSON-RPC-API, damit auch Skripte und KI-Agenten damit Video schneiden können.

## Highlights

- 🚫 **Keine Wasserzeichen. Kein Konto. Keine Bezahlschranke.** Niemals.
- 🔒 **100 % lokal.** Nichts wird hochgeladen. Funktioniert offline.
- 💬 **Automatische Untertitel.** Lokales Whisper. Modellgröße wählen, gestylte Untertitel auf der Zeitleiste bekommen.
- 🗣️ **Text-zu-Sprache + Stimmklonen.** Kostenlose lokale Stimmen, oder jede Stimme aus ein paar Sekunden Aufnahme.
- 🧍 **Hintergrundentfernung.** Personen, Objekte, oder die Maske selbst malen.
- 🎞️ **Keyframes.** Position, Skalierung, Drehung, Deckkraft, Lautstärke, Effektparameter. Kurveneditor eingebaut.
- ✨ **100 Effekte, Filter und Übergänge.** GPU-gerendert, live in der Vorschau.
- ✂️ **Schnell schneiden.** Teilen, trimmen, Ripple, zusammenführen, Standbild, Geschwindigkeit. Magnetische Zeitleiste, wenn du willst.
- 🎚️ **Mehrere Spuren, mehrere Zeitleisten.** Mehrere Schnitte in einem Projekt. Mischmodi, Zuschnitt, Spiegelungen.
- 📝 **Titel.** Schriften, Kontur, Schatten, Hintergrundplatte. Vorlagen zum Loslegen.
- 🎙️ **Stimmbereinigung mit einem Schalter.** Entrauschen, Stimme verbessern, Lautheit angleichen. Dazu Chipmunk, Roboter, Telefon und Freunde.
- 📤 **Export.** H.264, HEVC, AV1. Bis 4K 60, 10-Bit-Farbe.
- 🦀 **Native Rust-Engine.** GPU-Compositor, Proxys, Hardware-Decoding. 4K scrubbt flüssig.
- 🤖 **Skriptbar.** Eine JSON-RPC-API, mit der Skripte und KI-Agenten Video schneiden können. Ein Build aus dem Quellcode bringt zusätzlich gRPC und ein Kommandozeilenwerkzeug mit.
- 🖥️ **macOS, Windows, Linux, Android.** 25 Sprachen. Dieselbe App, dieselben Projektdateien.

## Download

Zwei Wege hinein:

1. **[Die Website](https://concatenate.pages.dev/#download)** gibt dir den passenden Build für deinen Rechner. Fang hier an.
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** hat jeden Build für jede Plattform, mit Installern, Paketen und Prüfsummen. Für den Fall, dass du selbst wählen willst.

Concat ist in der **Beta**: Es funktioniert, hat aber noch Kanten. [Sag Bescheid](https://github.com/jub0t/Concat/issues), wenn du eine findest.

**Plattformen**

- ✅ **Windows** · x86_64 (ARM-Builds sind pausiert, siehe [#326](https://github.com/jub0t/Concat/issues/326)). Ein Setup und eine `.msi`. Wenn SmartScreen einen unsignierten Build stoppt: **Weitere Informationen** › **Trotzdem ausführen**
- ✅ **macOS** · Intel und Apple Silicon. Wenn macOS einen unsignierten Build nicht öffnen will: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 und ARM. `.deb`, `.rpm`, `.AppImage` und ein Arch-Paket. Außerdem ein Flatpak auf **[Flatpark](https://flatpark.org/apps/app.concat.editor/)**, einem Community-Flatpak-Remote, das die x86_64-`.deb` jeder Version verpackt und mit ihr aktualisiert
- ✅ **Android** · Telefone und Tablets
- ✅ **iOS / iPadOS** · iPhone und iPad, per Sideload

✅ Unterstützt · 🚧 In Arbeit · 🧪 Noch zu testen

**Systemanforderungen** und die Größen der optionalen Modelle stehen auf [der Website](https://concatenate.pages.dev/guides/system-requirements).

**[Code signing policy](../CODE_SIGNING.md)**

## Mitmachen

> [!IMPORTANT]
> Am besten hilfst du, indem du dir einen Build von der [Releases](https://github.com/jub0t/Concat/releases)-Seite holst und ihn benutzt: finde, wo er bricht, und sag, wo er besser sein könnte.
>
> Bereit, Code zu schreiben? [CONTRIBUTING.md](../CONTRIBUTING.md) beschreibt die Einrichtung, den Aufbau des Baums, die auszuführenden Prüfungen und wie Beiträge lizenziert werden. Du steuerst Concat aus einem Skript, einem Dienst oder einem Agenten? [Die Entwicklerdokumentation](https://concatenate.pages.dev/docs) beschreibt die Concat-API und ihre Transporte: JSON-RPC und gRPC. [In dieser Discussion](https://github.com/jub0t/Concat/discussions/3) wurde das Projekt angekündigt.
> 
> Mitwirkende können sich auf dem Discord-Server eine `@Contributor`-Rolle holen: einfach danach fragen.

## Sternverlauf

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
</a>

## Mitwirkende

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## DMCA

Wenn du glaubst, dass etwas in diesem Repository, in seinen Releases oder in einem Paket oder Modell, das es verbreitet, dein Urheberrecht verletzt, schreib eine E-Mail an [jub0trd@gmail.com](mailto:jub0trd@gmail.com) mit einer Beschreibung des Werks, das dir gehört, einem Link zu der Stelle, an der das verletzende Material hier erscheint, deinen Kontaktdaten, einer Erklärung, dass du in gutem Glauben annimmst, dass die Nutzung weder von dir noch von deinem Vertreter noch vom Gesetz erlaubt ist, und einer Erklärung unter Strafandrohung für Meineid, dass die Mitteilung zutrifft und dass du der Rechteinhaber oder befugt bist, für ihn zu handeln, versehen mit deiner handschriftlichen oder elektronischen Unterschrift. Gültige Mitteilungen werden umgehend bearbeitet, und wessen Material entfernt wird, kann eine Gegendarstellung (Counter-Notice) an dieselbe Adresse schicken.
