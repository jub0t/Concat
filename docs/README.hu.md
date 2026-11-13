<p align="center">
  <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark-dark.png" alt="Concat" width="100" />
</p>

<h1 align="center">Concat</h1>
<h3 align="center">Ingyenes, nyílt forráskódú videovágó minden platformra.</h3>

<div align="center">

<img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/editor.png" alt="Concat editor" width="100%" />

<p align="center">
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/github/downloads/jub0t/concat/total?style=flat&logo=github&logoColor=F8F8F8&label=Downloads&labelColor=212123&color=0568FD" alt="Total Downloads" /></a>
  <a href="https://github.com/jub0t/Concat/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/jub0t/Concat/ci.yml?style=flat&logo=githubactions&logoColor=F8F8F8&label=Build&labelColor=000000" alt="Build Status" /></a>
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/badge/Version-0.2.9-0568FD?style=flat&logo=semver&logoColor=F8F8F8&labelColor=212123" alt="Concat Version 0.2.9" /></a>
  <a href="https://discord.gg/DVuPfpXfqP"><img src="https://img.shields.io/badge/Discord-Join%20the%20server-5865F2?style=flat&logo=discord&logoColor=F8F8F8&labelColor=000000" alt="Join Concat Discord" /></a>
  <a href="../LICENSE"><img src="https://img.shields.io/badge/License-AGPL%20v3-0568FD?style=flat&logo=gnu&logoColor=F8F8F8&labelColor=212123" alt="License: AGPL-3.0-or-later" /></a>
</p>

<p align="center">
  <a href="https://www.star-history.com/jub0t/concat"><img src="https://api.star-history.com/badge?repo=jub0t/concat&type=rank" alt="Star History Rank" height="62" /></a>
  <a href="https://trendshift.io/repositories/191379?utm_source=trendshift-badge&amp;utm_medium=badge&amp;utm_campaign=badge-trendshift-191379" target="_blank" rel="noopener noreferrer"><img src="https://trendshift.io/api/badge/trendshift/repositories/191379/daily?language=Rust" alt="jub0t%2FConcat | Trendshift" width="250" height="55"/></a>
  <a href="https://concat.standalones.org/#download"><img src="../assets/download_button.svg" alt="Download Concat" width="251" /></a>
</p>

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇳🇱 Nederlands](README.nl.md) · [🇵🇱 Polski](README.pl.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇦🇿 Azərbaycanca](README.az.md) · [🇭🇷 Hrvatski](README.hr.md) · **🇭🇺 Magyar** · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇹🇭 ไทย](README.th.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md) · [🇧🇩 বাংলা](README.bn.md)

> Az angol README-ből fordítva 2026. november 5-én. Az angol változat az, amelyet naprakészen tartunk.

## Fizetett szponzorok

<table width="100%">
  <tr>
    <td align="center">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>Használatalapú proxyk fejlesztőknek<br />A <code>JUB0T</code> kóddal 20% kedvezmény jár</sub>
    </td>
  </tr>
</table>

</div>


## A projektről

A Concat ingyenes, nyílt forráskódú videovágó macOS-re, Windowsra, Linuxra és Androidra. Lefedi, amire a legtöbb vágáshoz szükség van: automatikus feliratok, szövegfelolvasás, háttér-eltávolítás, kulcskockás animáció, effektek és címek, többsávos vágás, 4K-s exportálás. Nincs apró betű: nincs vízjel, nincs fiók, nincs előfizetés, nincs feltöltés.

Minden helyben fut, egy natív Rust motoron, GPU-s kompozitorral. Telepítsd, húzd be a felvételeket, vágj. A feliratokhoz, hangokhoz és kivágáshoz használt MI-modellek egyszer töltődnek le a Beállításokból, utána offline működnek. A felvételeid sosem hagyják el a lemezedet.

**Jó ehhez:** TikTok, Reels és Shorts, YouTube-videók, oktatóanyagok és képernyőfelvételek, podcastrészletek, mémek.

**És gépeknek is:** JSON-RPC API, így szkriptek és MI-ügynökök is vághatnak vele videót.

## Kiemelt funkciók

- 🚫 **Nincs vízjel. Nincs fiók. Nincs fizetés.** Soha.
- 🔒 **100%-ban helyi.** Semmi nem töltődik fel. Offline is működik.
- 💬 **Automatikus feliratok.** Helyi Whisper. Válassz modellméretet, és formázott feliratokat kapsz az idővonalon.
- 🗣️ **Szövegfelolvasás + hangklónozás.** Ingyenes helyi hangok, vagy bármilyen hang néhány másodpercnyi felvételből.
- 🧍 **Háttér-eltávolítás.** Személyek, tárgyak, vagy rajzold meg magad a maszkot.
- 🎞️ **Kulcskockák.** Pozíció, méretarány, forgatás, átlátszatlanság, hangerő, effektparaméterek. Beépített görbeszerkesztő.
- ✨ **100 effekt, szűrő és áttűnés.** GPU-n renderelve, élőben az előnézetben.
- ✂️ **Gyors vágás.** Vágás, megvágás, ripple, egyesítés, kimerevítés, sebesség. Mágneses idővonal, ha szeretnéd.
- 🎚️ **Több sáv, több idővonal.** Több vágás egy projektben. Keverési módok, körbevágás, tükrözés.
- 📝 **Címek.** Betűtípusok, körvonal, árnyék, háttérdoboz. Sablonok a kezdéshez.
- 🎙️ **Hangtisztítás egy kapcsolóval.** Zajcsökkentés, hangjavítás, hangosság-normalizálás. Plusz mókus, robot, telefon és társaik.
- 📤 **Exportálás.** H.264, HEVC, AV1. Akár 4K 60, 10 bites szín.
- 🦀 **Natív Rust motor.** GPU-s kompozitor, proxyk, hardveres dekódolás. A 4K is akadásmentesen játszható le.
- 🤖 **Szkriptelhető.** JSON-RPC API, amellyel szkriptek és MI-ügynökök vághatnak videót. A forrásból fordított változat gRPC-t és parancssori eszközt is ad.
- 🖥️ **macOS, Windows, Linux, Android.** 27 nyelv. Ugyanaz az alkalmazás, ugyanazok a projektfájlok.

## Letöltés

Két út:

1. **[A weboldal](https://concat.standalones.org/#download)** a számítógépednek megfelelő változatot adja. Itt kezdj.
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** minden platformra minden változatot tartalmaz, telepítőkkel, csomagokkal és ellenőrzőösszegekkel. Ha magad szeretnél választani.

A Concat **béta**: működik, de még vannak durva szélei. [Szólj](https://github.com/jub0t/Concat/issues), ha belefutsz egybe.

**Platformok**

- ✅ **Windows** · x86_64 (az ARM-változatok szünetelnek, lásd [#326](https://github.com/jub0t/Concat/issues/326)). Telepítő és `.msi`. Ha a SmartScreen megállít egy aláíratlan változatot: **További információ** › **Futtatás mindenképp**
- ✅ **macOS** · Intel és Apple Silicon. Ha a macOS nem hajlandó megnyitni egy aláíratlan változatot: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 és ARM. `.deb`, `.rpm`, `.AppImage` és Arch-csomag. Flatpakként is elérhető a **[Flatparkon](https://flatpark.org/apps/app.concat.editor/)**, egy közösségi Flatpak-tárolóban, amely minden kiadás x86_64-es `.deb` csomagját becsomagolja, és együtt frissül vele
- ✅ **Android** · telefonok és táblagépek
- ✅ **iOS / iPadOS** · iPhone és iPad, sideloaddal

✅ Támogatott · 🚧 Fejlesztés alatt · 🧪 Tesztelésre

A **rendszerkövetelmények** és az opcionális modellek mérete a [weboldalon](https://concat.standalones.org/guides/system-requirements) található.

**[Code signing policy](../CODE_SIGNING.md)**

## Közreműködés

> [!IMPORTANT]
> A legjobb módja a közreműködésnek, ha letöltesz egy változatot a [Releases](https://github.com/jub0t/Concat/releases) oldalról, és használod: megtalálod, hol törik el, és elmondod, hol lehetne jobb.
>
> Készen állsz kódot írni? A [CONTRIBUTING.md](../CONTRIBUTING.md) leírja a beállítást, a forrásfa felépítését, a futtatandó ellenőrzéseket és a hozzájárulások licencelését. Szkriptből, szolgáltatásból vagy ügynökből vezérled a Concatot? A [fejlesztői dokumentáció](https://concat.standalones.org/docs) bemutatja a Concat API-t és átviteleit: JSON-RPC és gRPC. [Ebben a beszélgetésben](https://github.com/jub0t/Concat/discussions/3) jelentették be a projektet.
> 
> A közreműködők kérhetik a `@Contributor` szerepet a Discord-szerveren: csak szólj.

## Csillagok története

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
</a>

## Közreműködők

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## DMCA

Ha úgy gondolod, hogy valami ebben a tárolóban, a kiadásaiban vagy az általa terjesztett csomagban vagy modellben sérti a szerzői jogodat, írj e-mailt a [jub0trd@gmail.com](mailto:jub0trd@gmail.com) címre, amelyben leírod a tulajdonodban lévő művet, megadod a hivatkozást arra a helyre, ahol a kifogásolt anyag itt megjelenik, a kapcsolattartási adataidat, egy nyilatkozatot arról, hogy jóhiszeműen úgy véled, a felhasználást sem te, sem a képviselőd, sem a jogszabály nem engedélyezte, valamint egy hamis nyilatkozat esetén büntetőjogi felelősséged tudatában tett nyilatkozatot arról, hogy az értesítés pontos, és te vagy a jogtulajdonos, vagy jogosult vagy a nevében eljárni, a saját kezű vagy elektronikus aláírásoddal ellátva. Az érvényes értesítéseket haladéktalanul feldolgozzuk, és bárki, akinek az anyagát eltávolították, ugyanerre a címre küldhet ellenértesítést.
