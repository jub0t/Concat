<p align="center">
  <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark-dark.png" alt="Concat" width="100" />
</p>

<h1 align="center">Concat</h1>
<h3 align="center">Der kostenlose, quelloffene Videoeditor für alle Plattformen.</h3>

<div align="center">

<img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/editor.png" alt="Concat editor" width="100%" />

<p align="center">
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/github/downloads/jub0t/concat/total?style=flat&logo=github&logoColor=F8F8F8&label=Downloads&labelColor=212123&color=0568FD" alt="Total Downloads" /></a>
  <a href="https://github.com/jub0t/Concat/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/jub0t/Concat/ci.yml?style=flat&logo=githubactions&logoColor=F8F8F8&label=Build&labelColor=000000" alt="Build Status" /></a>
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/badge/Version-0.2.13-0568FD?style=flat&logo=semver&logoColor=F8F8F8&labelColor=212123" alt="Concat Version 0.2.13" /></a>
  <a href="https://discord.gg/DVuPfpXfqP"><img src="https://img.shields.io/badge/Discord-Join%20the%20server-5865F2?style=flat&logo=discord&logoColor=F8F8F8&labelColor=000000" alt="Join Concat Discord" /></a>
  <a href="../LICENSE"><img src="https://img.shields.io/badge/License-AGPL%20v3-0568FD?style=flat&logo=gnu&logoColor=F8F8F8&labelColor=212123" alt="License: AGPL-3.0-or-later" /></a>
</p>

<p align="center">
  <a href="https://www.star-history.com/jub0t/concat"><img src="https://api.star-history.com/badge?repo=jub0t/concat&type=rank" alt="Star History Rank" height="62" /></a>
  <a href="https://concat.standalones.org/#download"><img src="../assets/download_button.svg" alt="Download Concat" width="251" /></a>
  <a href="https://trendshift.io/repositories/191379?utm_source=repository-badge&amp;utm_medium=badge&amp;utm_campaign=badge-repository-191379" target="_blank" rel="noopener noreferrer"><img src="https://trendshift.io/api/badge/repositories/191379" alt="jub0t%2FConcat | Trendshift" width="250" height="55"/></a>
</p>

[🇬🇧 English](../README.md) · **🇩🇪 Deutsch** · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇳🇱 Nederlands](README.nl.md) · [🇵🇱 Polski](README.pl.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇦🇿 Azərbaycanca](README.az.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇭🇺 Magyar](README.hu.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇹🇭 ไทย](README.th.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md) · [🇧🇩 বাংলা](README.bn.md)

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

Concat ist ein kostenloser, quelloffener Videoeditor für macOS, Windows, Linux und Android. Er kann, was die meisten Schnitte brauchen: automatische Untertitel, Stimmisolierung, Text-zu-Sprache, Hintergrundentfernung, Animationen und Keyframes, Effekte und Titel, Schnitt auf mehreren Spuren, 4K-Export. Ohne die Haken: kein Wasserzeichen, kein Konto, kein Abo, kein Upload.

Alles läuft lokal auf einer nativen Rust-Engine mit GPU-Compositor. Installieren, Material hineinziehen, schneiden. Die KI-Modelle für Untertitel, Stimmen und Freisteller werden einmal in den Einstellungen geladen und arbeiten danach offline. Dein Material verlässt deine Festplatte nie.

**Gut für:** TikTok, Reels und Shorts, YouTube-Videos, Tutorials und Bildschirmaufnahmen, Podcast-Clips, Memes.

**Auch für Maschinen:** eine JSON-RPC-API, damit auch Skripte und KI-Agenten damit Video schneiden können.

## Highlights

- 🚫 **Keine Wasserzeichen. Kein Konto. Keine Bezahlschranke.** Niemals.
- 🔒 **100 % lokal.** Nichts wird hochgeladen, auch die KI nicht. Funktioniert offline.
- 💬 **Automatische Untertitel.** In der Sprache, die du sprichst, auf deinem Rechner erzeugt, gestylt und auf der Zeitleiste.
- 🎤 **Stimme isolieren.** Hol eine Stimme aus Musik oder Lärm heraus, oder nimm sie heraus, jeder Teil mit eigener Lautstärke.
- 🗣️ **Text-zu-Sprache + Stimmklonen.** Eine kostenlose lokale Stimme, oder jede Stimme aus ein paar Sekunden Aufnahme. Auf dem Desktop.
- 🧍 **Hintergrundentfernung, Chroma-Key und Masken.** Personen und Objekte werden für dich freigestellt, ein Greenscreen, der sich selbst keyt, und ein Stapel Masken auf jedem Clip, jede mit eigenem Modus und eigenen Keyframes.
- 🌀 **Ein-, Aus- und Schleifen-Animationen.** Für Clips, Bilder, Titel und Formen. Jede spielt auf ihrer Karte ab, bevor du sie wählst.
- ✨ **Über 100 Effekte, Filter und Übergänge, 25 Soundeffekte.** Filmische Looks, Wackler und Einschläge, Retro, Wischer, alles live in der Vorschau. Ein Übergang ist ein Clip, den du wie jeden anderen verschiebst und trimmst.
- 🎞️ **Keyframes.** Position, Skalierung, Größe, Drehung, Deckkraft, Lautstärke, Effektparameter. Kurveneditor eingebaut.
- 🏃 **Tempo-Rampen und -Kurven.** Konstant oder gezeichnet, glatt oder gerade. Standbilder und Rückwärts.
- 🪄 **Verbessern und Stabilisieren.** Weiches oder altes Material auffrischen, bis 2K. Ein verwackeltes Bild beruhigen.
- ✂️ **Schnell schneiden, genau schneiden.** Teilen, Trimmen, Ripple, Zusammenführen, verknüpfte Untertitel und Overlays, auf Wunsch eine magnetische Zeitleiste. Jeder Schnitt sitzt auf dem Bild, in Vorschau und Export gleich.
- 🎚️ **Mehrere Spuren, mehrere Zeitleisten.** Video, Standbilder, Titel und Formen auf beliebig vielen Spuren stapeln. Mehrere Schnittfassungen in einem Projekt. Mischmodi, Zuschneiden, Spiegeln.
- 📝 **Titel und Formen.** Schriften, Kontur, Schatten, Hintergrundplatten, farbige Emojis und Vorlagen für Arabisch, Chinesisch, Hindi, Thai und acht weitere Sprachen. Formen mit Verläufen, Sternen, Umrissen und Schatten.
- 🎙️ **Voiceover und Stimmbereinigung mit einem Schalter.** Sprich über deinen Schnitt. Rauschen entfernen, Stimme verbessern, Lautstärke angleichen. Dazu Chipmunk, Roboter, Telefon und Co.
- 📤 **Export.** H.264, HEVC, AV1, mit dem eigenen Encoder deines Computers. Bis zu 4K 60, 10-Bit-Farbe.
- ⚡ **Schnell.** Proxys eingebaut, damit 4K flüssig scrubbt. Titel werden auf allen Kernen gezeichnet, und jeder Effekt ist bereit, sobald sich das Fenster öffnet.
- 🛟 **Läuft auf jedem Rechner weiter.** Wenn der Grafikchip aufgibt, zeichnet die Vorschau weiter.
- 🤖 **Skriptbar.** Eine JSON-RPC-API, mit der Skripte und KI-Agenten Video schneiden können. Ein Build aus dem Quellcode bringt zusätzlich ein Kommandozeilenwerkzeug mit.
- 🖥️ **macOS, Windows, Linux, Android.** 27 Sprachen. Dieselbe App, dieselben Projektdateien.

## Download

Zwei Wege hinein:

1. **[Die Website](https://concat.standalones.org/#download)** gibt dir den passenden Build für deinen Rechner. Fang hier an.
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** hat jeden Build für jede Plattform, mit Installern, Paketen und Prüfsummen. Für den Fall, dass du selbst wählen willst.

Concat ist in der **Beta**: Es funktioniert, hat aber noch Kanten. [Sag Bescheid](https://github.com/jub0t/Concat/issues), wenn du eine findest.

**Plattformen**

- ✅ **Windows** · x86_64 (ARM-Builds sind pausiert, siehe [#326](https://github.com/jub0t/Concat/issues/326)). Ein Setup und eine `.msi`. Wenn SmartScreen einen unsignierten Build stoppt: **Weitere Informationen** › **Trotzdem ausführen**
- ✅ **macOS** · Intel und Apple Silicon. Wenn macOS einen unsignierten Build nicht öffnen will: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 und ARM. `.deb`, `.rpm`, `.AppImage` und ein Arch-Paket. Außerdem ein Flatpak auf **[Flatpark](https://flatpark.org/apps/app.concat.editor/)**, einem Community-Flatpak-Remote, das die x86_64-`.deb` jeder Version verpackt und mit ihr aktualisiert
- ✅ **Android** · Telefone und Tablets

✅ Unterstützt · 🚧 In Arbeit · 🧪 Noch zu testen

**Systemanforderungen** und die Größen der optionalen Modelle stehen auf [der Website](https://concat.standalones.org/guides/system-requirements).

**[Code signing policy](../CODE_SIGNING.md)**

## Mitmachen

> [!IMPORTANT]
> Am besten hilfst du, indem du dir einen Build von der [Releases](https://github.com/jub0t/Concat/releases)-Seite holst und ihn benutzt: finde, wo er bricht, und sag, wo er besser sein könnte.
>
> Bereit, Code zu schreiben? [CONTRIBUTING.md](../CONTRIBUTING.md) beschreibt die Einrichtung, den Aufbau des Baums, die auszuführenden Prüfungen und wie Beiträge lizenziert werden. Du steuerst Concat aus einem Skript, einem Dienst oder einem Agenten? [Die Entwicklerdokumentation](https://concat.standalones.org/docs) beschreibt die Concat-API und ihren Transport: JSON-RPC. [In dieser Discussion](https://github.com/jub0t/Concat/discussions/3) wurde das Projekt angekündigt.
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
