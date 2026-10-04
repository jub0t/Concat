<div align="center">
<table width="100%">
  <tr>
    <td align="left" width="120">
      <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark.png" alt="Concat" width="100" />
    </td>
    <td align="">
      <h1>Concat</h1>
      <h3 style="margin-top: -10px;">Istinski besplatna zamjena za CapCut otvorenog koda, za sve platforme.</h3>
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

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · **🇭🇷 Hrvatski** · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md)

> Prevedeno s engleskog README-a 2. listopada 2026. Engleska je verzija ona koja se održava ažurnom.

## Plaćeni sponzori

<table width="100%">
  <tr>
    <td align="center" width="50%">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>Proxyji s plaćanjem po potrošnji za razvojne programere<br />Kod <code>JUB0T</code> daje 20 % popusta</sub>
    </td>
    <td align="center" width="50%">
      <a href="#sponsoring"><b>Vaš logo ovdje</b></a><br />
      <sub><a href="#sponsoring">Sponzorirajte Concat</a> i vaše ime, logo i poveznica zauzimaju ovo mjesto</sub>
    </td>
  </tr>
</table>

</div>


## O projektu

Concat je besplatan videouređivač otvorenog koda i alternativa CapCutu za macOS, Windows, Linux i Android. Pokriva ono zbog čega ljudi zapravo otvaraju CapCut: automatske titlove, pretvorbu teksta u govor, uklanjanje pozadine, animaciju ključnim kadrovima, efekte i naslove, višetračno rezanje, izvoz u 4K. Bez ikakvih kvaka: bez vodenog žiga, bez računa, bez pretplate, bez učitavanja.

Sve se izvodi lokalno na nativnom Rust motoru s GPU kompozitorom. Instalirajte, ubacite snimke, režite. AI modeli za titlove, glasove i izrezivanje preuzimaju se jednom iz Postavki i nakon toga rade izvan mreže. Vaše snimke nikada ne napuštaju vaš disk.

**Dobro za:** TikTok, Reels i Shorts, YouTube videe, tutorijale i snimke zaslona, isječke podcasta, memeove.

**I za strojeve:** JSON-RPC, gRPC i MCP API, pa skripte i AI agenti također mogu njime rezati video.

## Istaknuto

- 🚫 **Bez vodenih žigova. Bez računa. Bez naplate.** Nikad.
- 🔒 **100 % lokalno.** Ništa se ne učitava. Radi izvan mreže.
- 💬 **Automatski titlovi.** Lokalni Whisper. Odaberite veličinu modela i dobijete stilizirane titlove na vremenskoj crti.
- 🗣️ **Tekst u govor + kloniranje glasa.** Besplatni lokalni glasovi ili bilo koji glas iz nekoliko sekundi snimke.
- 🧍 **Uklanjanje pozadine.** Osobe, predmeti ili sami nacrtajte masku.
- 🎞️ **Ključni kadrovi.** Položaj, skala, rotacija, prozirnost, glasnoća, parametri efekata. Ugrađeni uređivač krivulja.
- ✨ **Više od 170 efekata, filtara, prijelaza i animacija teksta.** Iscrtano na GPU-u, uživo u pregledu.
- ✂️ **Brzo rezanje.** Podijeli, skrati, ripple, spoji, zamrzni kadar, brzina. Magnetska vremenska crta ako je želite.
- 🎚️ **Više traka, više vremenskih crta.** Više montaža u jednom projektu. Načini stapanja, obrezivanje, zrcaljenja.
- 📝 **Naslovi.** Fontovi, obrub, sjena, pozadinska pločica. Predlošci za početak.
- 🎙️ **Čišćenje glasa jednim prekidačem.** Uklanjanje šuma, poboljšanje glasa, izjednačavanje glasnoće. Plus vjeverica, robot, telefon i društvo.
- 📤 **Izvoz.** H.264, HEVC, AV1. Do 4K 60, 10-bitna boja.
- 🦀 **Nativni Rust motor.** GPU kompozitor, proxyji, hardversko dekodiranje. 4K se pregledava glatko.
- 🤖 **Skriptabilan.** JSON-RPC, gRPC i MCP API, plus CLI. AI agenti mogu njime rezati video.
- 🖥️ **macOS, Windows, Linux, Android.** 14 jezika. Ista aplikacija, iste datoteke projekta.

## Preuzimanje

Dva ulaza:

1. **[Web-stranica](https://concatenate.pages.dev/#download)** daje vam pravu inačicu za vaše računalo. Počnite ovdje.
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** ima svaku inačicu za svaku platformu, s instalacijama, paketima i kontrolnim zbrojevima. Za kad želite birati.

Concat je u **beti**: radi, a još ima rubova. [Javite](https://github.com/jub0t/Concat/issues) kad naiđete na jedan.

**Platforme**

- ✅ **Windows** · x86_64 i ARM. Instalacija i `.msi`. Ako SmartScreen zaustavi nepotpisanu inačicu: **Više informacija** › **Svejedno pokreni**
- ✅ **macOS** · Intel i Apple Silicon. Ako macOS odbije otvoriti nepotpisanu inačicu: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 i ARM. `.deb`, `.rpm`, `.AppImage` i Arch paket. Također Flatpak na **[Flatparku](https://flatpark.org/apps/app.concat.editor/)**, Flatpak repozitoriju zajednice koji omata x86_64 `.deb` svakog izdanja i ažurira se s njim
- ✅ **Android** · telefoni i tableti
- ✅ **iOS / iPadOS** · iPhone i iPad, sideloadom

✅ Podržano · 🚧 U izradi · 🧪 Za testiranje

**Zahtjevi sustava** i veličine neobaveznih modela nalaze se na [web-stranici](https://concatenate.pages.dev/guides/system-requirements).

## Prvi koraci

Preuzmite, otvorite, ubacite snimke, režite. Bez računa, bez postavljanja.

**Ako nešto prijavljujete:** svako pokretanje zapisuje dnevnik, a u Postavke › O programu nalazi se gumb koji ga otvara, uz onaj koji kopira podatke o vašem sustavu. Priložite oboje uz [issue](https://github.com/jub0t/Concat/issues) i prijava stiže sa svime što joj treba. Čuva se zadnjih deset pokretanja, pa je i jučerašnje još tu; ništa se nikada ne šalje nikamo samo od sebe.

## Kako doprinijeti

> [!IMPORTANT]
> Najbolji način doprinosa jest uzeti inačicu sa stranice [Releases](https://github.com/jub0t/Concat/releases) i koristiti je: pronaći gdje puca i reći gdje bi mogla biti bolja.
>
> Spremni pisati kod? [CONTRIBUTING.md](../CONTRIBUTING.md) pokriva postavljanje, raspored stabla, provjere koje treba pokrenuti i kako se doprinosi licenciraju. Upravljate Concatom iz skripte, servisa ili agenta? [Dokumentacija za razvojne programere](https://concatenate.pages.dev/docs) pokriva Concat API i njegove prijenose: JSON-RPC, gRPC i MCP. [U ovoj raspravi](https://github.com/jub0t/Concat/discussions/3) projekt je najavljen.
> 
> Suradnici mogu zatražiti ulogu `@Contributor` na Discord poslužitelju, samo pitajte.

## Suradnici

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## Povijest zvjezdica

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&theme=dark&legend=bottom-right" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
 </picture>
</a>

<a id="sponsoring"></a>
## Sponzoriranje

Concat nema naplatu i nikada je neće imati: bez vodenog žiga, bez računa, bez plaćene razine. Sponzoriranje stoji umjesto toga. Ako je Concat za vas zamijenio pretplatu, djelić nje održava ga na životu.

**Kamo odlazi**

[Plan razvoja](https://concatenate.pages.dev/roadmap) prikazuje što sponzorstvo plaća i koliko koja stavka stoji.

Odaberite razinu na [web-stranici](https://concatenate.pages.dev/#sponsor).

<a href="https://concatenate.pages.dev/#sponsor"><img src="https://img.shields.io/badge/Sponsor-Concat-0568FD?style=flat-square&labelColor=212123" alt="Sponsor Concat on the website" /></a>

Niste u mogućnosti pridonijeti? Zvjezdica, prijava greške ili riječ nekome tko montira video također puno znače.
