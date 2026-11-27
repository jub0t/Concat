<p align="center">
  <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark-dark.png" alt="Concat" width="100" />
</p>

<h1 align="center">Concat</h1>
<h3 align="center">Besplatan videouređivač otvorenog koda, za sve platforme.</h3>

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

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇳🇱 Nederlands](README.nl.md) · [🇵🇱 Polski](README.pl.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇦🇿 Azərbaycanca](README.az.md) · **🇭🇷 Hrvatski** · [🇭🇺 Magyar](README.hu.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇹🇭 ไทย](README.th.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md) · [🇧🇩 বাংলা](README.bn.md)

> Prevedeno s engleskog README-a 11. listopada 2026. Engleska je verzija ona koja se održava ažurnom.

## Plaćeni sponzori

<table width="100%">
  <tr>
    <td align="center">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>Proxyji s plaćanjem po potrošnji za razvojne programere<br />Kod <code>JUB0T</code> daje 20 % popusta</sub>
    </td>
  </tr>
</table>

</div>


## O projektu

Concat je besplatan videouređivač otvorenog koda za macOS, Windows, Linux i Android. Pokriva ono što treba većini montaža: automatske titlove, izdvajanje glasa, pretvorbu teksta u govor, uklanjanje pozadine, animacije i ključne kadrove, efekte i naslove, višetračno rezanje, izvoz u 4K. Bez ikakvih kvaka: bez vodenog žiga, bez računa, bez pretplate, bez učitavanja.

Sve se izvodi lokalno na nativnom Rust motoru s GPU kompozitorom. Instalirajte, ubacite snimke, režite. AI modeli za titlove, glasove i izrezivanje preuzimaju se jednom iz Postavki i nakon toga rade izvan mreže. Vaše snimke nikada ne napuštaju vaš disk.

**Dobro za:** TikTok, Reels i Shorts, YouTube videe, tutorijale i snimke zaslona, isječke podcasta, memeove.

**I za strojeve:** JSON-RPC API, pa skripte i AI agenti također mogu njime rezati video.

## Istaknuto

- 🚫 **Bez vodenih žigova. Bez računa. Bez naplate.** Nikad.
- 🔒 **100 % lokalno.** Ništa se ne učitava, ni umjetna inteligencija. Radi izvan mreže.
- 💬 **Automatski titlovi.** Na jeziku kojim govorite, napravljeni na vašem računalu, stilizirani i odmah na vremenskoj crti.
- 🎤 **Izdvajanje glasa.** Izvucite glas iz glazbe ili buke, ili ga uklonite, svaki dio na svojoj razini.
- 🗣️ **Pretvorba teksta u govor + kloniranje glasa.** Besplatan lokalni glas ili bilo koji glas iz nekoliko sekundi snimke. Na računalu.
- 🧍 **Uklanjanje pozadine, chroma key i maske.** Ljudi i objekti izrezani umjesto vas, zeleni zaslon koji se sam uklanja i hrpa maski na svakom klipu, svaka sa svojim načinom i ključnim kadrovima.
- 🌀 **Animacije ulaza, izlaza i petlje.** Za klipove, slike, naslove i oblike. Svaka se reproducira na svojoj kartici prije nego što je odaberete.
- ✨ **Više od 100 efekata, filtara i prijelaza, 25 zvučnih efekata.** Filmski izgledi, tresnje i udari, retro, brisanja, sve uživo u pregledu. Prijelaz je klip koji pomičete i skraćujete kao i svaki drugi.
- 🎞️ **Ključni kadrovi.** Položaj, mjerilo, veličina, rotacija, neprozirnost, glasnoća, parametri efekata. Uređivač krivulja je ugrađen.
- 🏃 **Rampe i krivulje brzine.** Stalne ili nacrtane, glatke ili ravne. Zamrznuti kadrovi i obrnuto.
- 🪄 **Poboljšaj i Stabiliziraj.** Obnovite mutan ili stari snimak, do 2K. Smirite drhtav kadar.
- ✂️ **Režite brzo, režite točno.** Podijeli, skrati, ripple, spoji, povezani titlovi i preklapanja, magnetska vremenska crta ako je želite. Svaki rez pada točno na kadar, jednako u pregledu i u izvozu.
- 🎚️ **Višetračno, s više vremenskih crta.** Slažite video, slike, naslove i oblike na koliko god tračaka želite. Više rezova u jednom projektu. Načini stapanja, obrezivanje, zrcaljenje.
- 📝 **Naslovi i oblici.** Fontovi, obrub, sjena, podloge, emotikoni u boji i predlošci za arapski, kineski, hindi, tajlandski i još osam jezika. Oblici s gradijentima, zvijezde, obrisi i sjene.
- 🎙️ **Govorni komentar i čišćenje glasa jednim prekidačem.** Snimajte preko svog reza. Uklanjanje šuma, poboljšanje glasa, izjednačavanje glasnoće. Plus vjeverica, robot, telefon i društvo.
- 📤 **Izvoz.** H.264, HEVC, AV1, na kodeku samog računala. Do 4K 60, 10-bitna boja.
- ⚡ **Brzo.** Proxyji su ugrađeni da se 4K glatko pomiče. Naslovi se crtaju na svim jezgrama, a svaki je efekt spreman čim se prozor otvori.
- 🛟 **Nastavlja raditi na svakom računalu.** Kad grafički čip odustane, pregled i dalje crta.
- 🤖 **Skriptabilan.** JSON-RPC API kojim skripte i AI agenti mogu rezati video. Inačica izgrađena iz izvornog koda dodaje alat za naredbeni redak.
- 🖥️ **macOS, Windows, Linux, Android.** 27 jezika. Ista aplikacija, iste datoteke projekata.

## Preuzimanje

Dva ulaza:

1. **[Web-stranica](https://concat.standalones.org/#download)** daje vam pravu inačicu za vaše računalo. Počnite ovdje.
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** ima svaku inačicu za svaku platformu, s instalacijama, paketima i kontrolnim zbrojevima. Za kad želite birati.

Concat je u **beti**: radi, a još ima rubova. [Javite](https://github.com/jub0t/Concat/issues) kad naiđete na jedan.

**Platforme**

- ✅ **Windows** · x86_64 (ARM inačice su pauzirane, vidi [#326](https://github.com/jub0t/Concat/issues/326)). Instalacija i `.msi`. Ako SmartScreen zaustavi nepotpisanu inačicu: **Više informacija** › **Svejedno pokreni**
- ✅ **macOS** · Intel i Apple Silicon. Ako macOS odbije otvoriti nepotpisanu inačicu: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 i ARM. `.deb`, `.rpm`, `.AppImage` i Arch paket. Također Flatpak na **[Flatparku](https://flatpark.org/apps/app.concat.editor/)**, Flatpak repozitoriju zajednice koji omata x86_64 `.deb` svakog izdanja i ažurira se s njim
- ✅ **Android** · telefoni i tableti

✅ Podržano · 🚧 U izradi · 🧪 Za testiranje

**Zahtjevi sustava** i veličine neobaveznih modela nalaze se na [web-stranici](https://concat.standalones.org/guides/system-requirements).

**[Code signing policy](../CODE_SIGNING.md)**

## Kako doprinijeti

> [!IMPORTANT]
> Najbolji način doprinosa jest uzeti inačicu sa stranice [Releases](https://github.com/jub0t/Concat/releases) i koristiti je: pronaći gdje puca i reći gdje bi mogla biti bolja.
>
> Spremni pisati kod? [CONTRIBUTING.md](../CONTRIBUTING.md) pokriva postavljanje, raspored stabla, provjere koje treba pokrenuti i kako se doprinosi licenciraju. Upravljate Concatom iz skripte, servisa ili agenta? [Dokumentacija za razvojne programere](https://concat.standalones.org/docs) pokriva Concat API i njegov prijenos: JSON-RPC. [U ovoj raspravi](https://github.com/jub0t/Concat/discussions/3) projekt je najavljen.
> 
> Suradnici mogu zatražiti ulogu `@Contributor` na Discord poslužitelju: samo pitajte.

## Povijest zvjezdica

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
</a>

## Suradnici

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## DMCA

Ako smatrate da nešto u ovom repozitoriju, njegovim izdanjima ili paketu ili modelu koji distribuira krši vaša autorska prava, pošaljite e-poruku na [jub0trd@gmail.com](mailto:jub0trd@gmail.com) s opisom djela čiji ste vlasnik, poveznicom na mjesto na kojem se ovdje pojavljuje sporni materijal, svojim podacima za kontakt, izjavom da u dobroj vjeri smatrate da tu upotrebu nisu odobrili ni vi, ni vaš zastupnik, ni zakon, te izjavom, pod prijetnjom kaznene odgovornosti za lažni iskaz, da je obavijest točna i da ste vlasnik ili ste ovlašteni djelovati u ime vlasnika, potpisanom vašim vlastoručnim ili elektroničkim potpisom. Valjane obavijesti obrađuju se bez odgode, a svatko čiji je materijal uklonjen može poslati protuobavijest na istu adresu.
