<div align="center">
<table width="100%">
  <tr>
    <td align="left" width="120">
      <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark.png" alt="Concat" width="100" />
    </td>
    <td align="">
      <h1>Concat</h1>
      <h3 style="margin-top: -10px;">De echt gratis, open-source en cross-platform vervanger voor CapCut.</h3>
    </td>
  </tr>
</table>

<img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/editor.png" alt="Concat editor" width="100%" />

<p align="center">
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/github/downloads/jub0t/concat/total?style=flat-square&logo=github&logoColor=F8F8F8&label=Downloads&labelColor=212123&color=0568FD" alt="Total Downloads" /></a>
  <a href="https://github.com/jub0t/Concat/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/jub0t/Concat/ci.yml?style=flat&logo=githubactions&logoColor=F8F8F8&label=Build&labelColor=000000" alt="Build Status" /></a>
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/badge/Version-0.2.7-0568FD?style=flat-square&logo=semver&logoColor=F8F8F8&labelColor=212123" alt="Concat Version 0.2.5" /></a>
  <a href="https://discord.gg/DVuPfpXfqP"><img src="https://img.shields.io/badge/Discord-Join%20the%20server-5865F2?style=flat&logo=discord&logoColor=F8F8F8&labelColor=000000" alt="Join Concat Discord" /></a>
  <a href="../LICENSE"><img src="https://img.shields.io/badge/License-AGPL%20v3-0568FD?style=flat-square&logo=gnu&logoColor=F8F8F8&labelColor=212123" alt="License: AGPL-3.0-or-later" /></a>
  <a href="https://concatenate.pages.dev/#sponsor"><img src="https://img.shields.io/badge/Sponsor-Concat-0568FD?style=flat-square&labelColor=212123" alt="Sponsor Concat" /></a>
</p>

<p align="center">
  <a href="https://concatenate.pages.dev/#download"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/download_button.svg" alt="Download Concat" width="220" /></a>
</p>

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · **🇳🇱 Nederlands** · [🇵🇱 Polski](README.pl.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇹🇭 ไทย](README.th.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md) · [🇧🇩 বাংলা](README.bn.md)

> Vertaald uit de Engelse README van 11 oktober 2026. De Engelse versie is degene die actueel wordt gehouden.

## Betaalde sponsors

<table width="100%">
  <tr>
    <td align="center" width="50%">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>Pay-as-you-go-proxy’s voor ontwikkelaars<br />Gebruik code <code>JUB0T</code> voor 20% korting</sub>
    </td>
    <td align="center" width="50%">
      <a href="https://concatenate.pages.dev/#sponsor"><b>Jouw logo hier</b></a><br />
      <sub><a href="https://concatenate.pages.dev/#sponsor">Sponsor Concat</a> en je naam, logo en link krijgen deze plek</sub>
    </td>
  </tr>
</table>

</div>


## Over Concat

Concat is een gratis, open-source video-editor en een alternatief voor CapCut op macOS, Windows, Linux en Android. Het doet waarvoor mensen CapCut eigenlijk openen: automatische ondertitels, tekst-naar-spraak, achtergrond verwijderen, keyframe-animatie, effecten en titels, monteren op meerdere sporen, exporteren in 4K. En dat zonder addertjes onder het gras: geen watermerk, geen account, geen abonnement, geen upload.

Alles draait lokaal, op een native Rust-engine met een GPU-compositor. Installeren, beeldmateriaal erin slepen, knippen. De AI-modellen voor ondertitels, stemmen en uitsnijden download je één keer via Instellingen, daarna werken ze offline. Je beeldmateriaal verlaat je schijf nooit.

**Geschikt voor:** TikTok, Reels en Shorts, YouTube-video’s, tutorials en schermopnames, podcastfragmenten, memes.

**Ook voor machines:** een JSON-RPC-, gRPC- en MCP-API, zodat ook scripts en AI-agents er video mee kunnen knippen.

## Hoogtepunten

- 🚫 **Geen watermerken. Geen account. Geen betaalmuur.** Nooit.
- 🔒 **100% lokaal.** Er wordt niets geüpload. Werkt offline.
- 💬 **Automatische ondertitels.** Lokale Whisper. Kies een modelgrootte en je krijgt opgemaakte ondertitels op de tijdlijn.
- 🗣️ **Tekst-naar-spraak + stemklonen.** Gratis lokale stemmen, of elke stem op basis van een paar seconden opname.
- 🧍 **Achtergrond verwijderen.** Mensen, objecten, of teken het masker zelf.
- 🎞️ **Keyframes.** Positie, schaal, rotatie, dekking, volume, effectparameters. Met ingebouwde curve-editor.
- ✨ **Meer dan 170 effecten, filters, overgangen en tekstanimaties.** Gerenderd op de GPU, live in het voorbeeld.
- ✂️ **Snel knippen.** Splitsen, trimmen, ripple, samenvoegen, stilstaand beeld, snelheid. Een magnetische tijdlijn als je dat wilt.
- 🎚️ **Meerdere sporen, meerdere tijdlijnen.** Verschillende montages in één project. Overvloeimodi, bijsnijden, spiegelen.
- 📝 **Titels.** Lettertypen, omlijning, schaduw, achtergrondvlak. Presets om mee te beginnen.
- 🎙️ **Stem opschonen met één schakelaar.** Ruis verwijderen, stem verbeteren, luidheid gelijktrekken. Plus chipmunk, robot, telefoon en meer.
- 📤 **Exporteren.** H.264, HEVC, AV1. Tot 4K 60, 10-bit kleur.
- 🦀 **Native Rust-engine.** GPU-compositor, proxy’s, hardwaredecodering. Zelfs 4K scrubt soepel.
- 🤖 **Scriptbaar.** JSON-RPC-, gRPC- en MCP-API, plus een CLI. AI-agents kunnen er video mee knippen.
- 🖥️ **macOS, Windows, Linux, Android.** 25 talen. Dezelfde app, dezelfde projectbestanden.

## Downloaden

Twee manieren:

1. **[De website](https://concatenate.pages.dev/#download)** geeft je de juiste build voor je computer. Begin hier.
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** heeft elke build voor elk platform, met installers, pakketten en checksums. Voor als je zelf wilt kiezen.

Concat is in **bèta**: het werkt, maar er zitten nog ruwe randjes aan. [Laat het weten](https://github.com/jub0t/Concat/issues) als je er een tegenkomt.

**Platforms**

- ✅ **Windows** · x86_64 en ARM. Een setup en een `.msi`. Als SmartScreen een niet-ondertekende build tegenhoudt: **Meer informatie** › **Toch uitvoeren**
- ✅ **macOS** · Intel en Apple silicon. Als macOS een niet-ondertekende build niet wil openen: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 en ARM. `.deb`, `.rpm`, `.AppImage` en een Arch-pakket. Ook als Flatpak op **[Flatpark](https://flatpark.org/apps/app.concat.editor/)**, een Flatpak-remote uit de community die de x86_64-`.deb` van elke release verpakt en met elke release meegaat
- ✅ **Android** · telefoons en tablets
- ✅ **iOS / iPadOS** · iPhone en iPad, via sideloading

✅ Ondersteund · 🚧 In ontwikkeling · 🧪 Nog te testen

**Systeemvereisten** en de groottes van de optionele modellen staan op [de website](https://concatenate.pages.dev/guides/system-requirements).

## Bijdragen

> [!IMPORTANT]
> De beste manier om bij te dragen is een build van de [Releases](https://github.com/jub0t/Concat/releases)-pagina te halen en die te gebruiken: zoek uit waar hij stukgaat, en zeg wat er beter kan.
>
> Klaar om code te schrijven? [CONTRIBUTING.md](../CONTRIBUTING.md) beschrijft de installatie, de indeling van de broncode, de controles die je moet draaien en onder welke licentie bijdragen vallen. Stuur je Concat aan vanuit een script, een service of een agent? [De ontwikkelaarsdocumentatie](https://concatenate.pages.dev/docs) beschrijft de Concat-API en de transports ervan: JSON-RPC, gRPC en MCP. In [deze Discussion](https://github.com/jub0t/Concat/discussions/3) werd het project aangekondigd.
> 
> Bijdragers kunnen een `@Contributor`-rol krijgen op de Discord-server, vraag er gewoon om.

## Bijdragers

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## Sterrengeschiedenis

<p align="center">
 <a href="https://www.star-history.com/jub0t/concat">
  <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/badge?repo=jub0t/concat&type=rank&theme=dark" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/badge?repo=jub0t/concat&type=rank" />
   <img alt="Star History Rank" src="https://api.star-history.com/badge?repo=jub0t/concat&type=rank" />
  </picture>
 </a>
</p>

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&theme=dark&legend=bottom-right" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
 </picture>
</a>

## DMCA

Als je denkt dat iets in deze repository, in de releases ervan, of in een pakket of model dat ermee wordt verspreid inbreuk maakt op jouw auteursrecht, stuur dan een e-mail naar [jub0trd@gmail.com](mailto:jub0trd@gmail.com) met een beschrijving van het werk waarvan je de rechthebbende bent, een link naar de plek waar het inbreukmakende materiaal hier staat, je contactgegevens, een verklaring dat je te goeder trouw meent dat het gebruik niet is toegestaan door jou, je vertegenwoordiger of de wet, en een verklaring, op straffe van meineed, dat de melding juist is en dat je de rechthebbende bent of bevoegd bent namens de rechthebbende op te treden, ondertekend met je handgeschreven of elektronische handtekening. Geldige meldingen worden snel afgehandeld, en iedereen van wie materiaal is verwijderd, kan een tegenmelding (counter-notice) naar hetzelfde adres sturen.
