<p align="center">
  <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark-dark.png" alt="Concat" width="100" />
</p>

<h1 align="center">Concat</h1>
<h3 align="center">Darmowy, otwartoźródłowy i wieloplatformowy zamiennik CapCuta.</h3>

<div align="center">

<img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/editor.png" alt="Concat editor" width="100%" />

<p align="center">
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/github/downloads/jub0t/concat/total?style=flat&logo=github&logoColor=F8F8F8&label=Downloads&labelColor=212123&color=0568FD" alt="Total Downloads" /></a>
  <a href="https://github.com/jub0t/Concat/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/jub0t/Concat/ci.yml?style=flat&logo=githubactions&logoColor=F8F8F8&label=Build&labelColor=000000" alt="Build Status" /></a>
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/badge/Version-0.2.8-0568FD?style=flat&logo=semver&logoColor=F8F8F8&labelColor=212123" alt="Concat Version 0.2.8" /></a>
  <a href="https://discord.gg/DVuPfpXfqP"><img src="https://img.shields.io/badge/Discord-Join%20the%20server-5865F2?style=flat&logo=discord&logoColor=F8F8F8&labelColor=000000" alt="Join Concat Discord" /></a>
  <a href="../LICENSE"><img src="https://img.shields.io/badge/License-AGPL%20v3-0568FD?style=flat&logo=gnu&logoColor=F8F8F8&labelColor=212123" alt="License: AGPL-3.0-or-later" /></a>
</p>

<p align="center">
  <a href="https://www.star-history.com/jub0t/concat"><img src="https://api.star-history.com/badge?repo=jub0t/concat&type=rank" alt="Star History Rank" height="62" /></a>
  <a href="https://trendshift.io/repositories/191379?utm_source=trendshift-badge&amp;utm_medium=badge&amp;utm_campaign=badge-trendshift-191379" target="_blank" rel="noopener noreferrer"><img src="https://trendshift.io/api/badge/trendshift/repositories/191379/daily?language=Rust" alt="jub0t%2FConcat | Trendshift" width="250" height="55"/></a>
  <a href="https://concatenate.pages.dev/#download"><img src="../assets/download_button.svg" alt="Download Concat" width="251" /></a>
</p>

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇳🇱 Nederlands](README.nl.md) · **🇵🇱 Polski** · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇦🇿 Azərbaycanca](README.az.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇹🇭 ไทย](README.th.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md) · [🇧🇩 বাংলা](README.bn.md)

> Przetłumaczono z angielskiego README z 11 października 2026 r. Na bieżąco aktualizowana jest wersja angielska.

## Płatni sponsorzy

<table width="100%">
  <tr>
    <td align="center">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>Proxy dla programistów, płatne za faktyczne użycie<br />Z kodem <code>JUB0T</code> 20% taniej</sub>
    </td>
  </tr>
</table>

</div>


## O Concat

Concat to darmowy, otwartoźródłowy edytor wideo i alternatywa dla CapCuta na systemy macOS, Windows, Linux i Android. Robi to, po co ludzie naprawdę otwierają CapCuta: automatyczne napisy, zamianę tekstu na mowę, usuwanie tła, animację na klatkach kluczowych, efekty i napisy tytułowe, montaż na wielu ścieżkach, eksport w 4K. I to bez żadnych haczyków: bez znaku wodnego, bez konta, bez subskrypcji, bez wysyłania plików.

Wszystko działa lokalnie, na natywnym silniku napisanym w Rust, z kompozytorem na GPU. Zainstaluj, wrzuć materiał, tnij. Modele AI do napisów, głosów i wycinania pobierasz raz w Ustawieniach, a potem działają offline. Twój materiał nigdy nie opuszcza dysku.

**Nadaje się do:** TikToka, Reels i Shorts, filmów na YouTube, poradników i nagrań ekranu, fragmentów podcastów, memów.

**Także dla maszyn:** API w JSON-RPC, dzięki któremu skrypty i agenci AI też mogą w nim montować wideo.

## Najważniejsze funkcje

- 🚫 **Bez znaków wodnych. Bez konta. Bez paywalla.** Nigdy.
- 🔒 **W 100% lokalnie.** Nic nie jest wysyłane. Działa offline.
- 💬 **Automatyczne napisy.** Lokalny Whisper. Wybierz rozmiar modelu i dostań sformatowane napisy na osi czasu.
- 🗣️ **Zamiana tekstu na mowę + klonowanie głosu.** Darmowe lokalne głosy albo dowolny głos z kilku sekund nagrania.
- 🧍 **Usuwanie tła.** Ludzie, obiekty albo maska, którą malujesz sam.
- 🎞️ **Klatki kluczowe.** Pozycja, skala, obrót, krycie, głośność, parametry efektów. Wbudowany edytor krzywych.
- ✨ **100 efektów, filtrów i przejść.** Renderowane na GPU, na żywo w podglądzie.
- ✂️ **Szybkie cięcie.** Dzielenie, przycinanie, ripple, scalanie, stopklatka, prędkość. Magnetyczna oś czasu, jeśli chcesz.
- 🎚️ **Wiele ścieżek, wiele osi czasu.** Kilka wersji montażu w jednym projekcie. Tryby mieszania, kadrowanie, odbicia.
- 📝 **Napisy tytułowe.** Czcionki, obrys, cień, tło pod tekstem. Gotowe szablony na start.
- 🎙️ **Czyszczenie głosu jednym przełącznikiem.** Odszumianie, poprawa głosu, wyrównanie głośności. Do tego wiewiórka, robot, telefon i spółka.
- 📤 **Eksport.** H.264, HEVC, AV1. Do 4K 60, kolor 10-bitowy.
- 🦀 **Natywny silnik w Rust.** Kompozytor na GPU, proxy, sprzętowe dekodowanie. Nawet 4K przewija się płynnie.
- 🤖 **Skryptowalny.** API w JSON-RPC, przez które skrypty i agenci AI mogą montować wideo. Wersja zbudowana ze źródeł dodaje gRPC i narzędzie wiersza poleceń.
- 🖥️ **macOS, Windows, Linux, Android.** 26 języków. Ta sama aplikacja, te same pliki projektów.

## Pobieranie

Dwie drogi:

1. **[Strona internetowa](https://concatenate.pages.dev/#download)** poda ci właściwą wersję dla twojego komputera. Zacznij tutaj.
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** mają każdą wersję na każdą platformę, z instalatorami, paczkami i sumami kontrolnymi. Na wypadek, gdy wolisz wybrać samodzielnie.

Concat jest w fazie **beta**: działa, ale wciąż ma swoje niedoróbki. [Daj znać](https://github.com/jub0t/Concat/issues), gdy na którąś trafisz.

**Platformy**

- ✅ **Windows** · x86_64 (wersje na ARM są wstrzymane, zobacz [#326](https://github.com/jub0t/Concat/issues/326)). Instalator i `.msi`. Jeśli SmartScreen zatrzyma niepodpisaną wersję: **Więcej informacji** › **Uruchom mimo to**
- ✅ **macOS** · Intel i Apple silicon. Jeśli macOS nie chce otworzyć niepodpisanej wersji: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 i ARM. `.deb`, `.rpm`, `.AppImage` i pakiet dla Archa. Do tego Flatpak na **[Flatpark](https://flatpark.org/apps/app.concat.editor/)**, społecznościowym repozytorium Flatpak, które opakowuje `.deb` dla x86_64 z każdego wydania i aktualizuje się razem z nim
- ✅ **Android** · telefony i tablety
- ✅ **iOS / iPadOS** · iPhone i iPad, przez sideloading

✅ Obsługiwane · 🚧 W trakcie prac · 🧪 Do przetestowania

**Wymagania systemowe** i rozmiary opcjonalnych modeli znajdziesz na [stronie](https://concatenate.pages.dev/guides/system-requirements).

**[Code signing policy](../CODE_SIGNING.md)**

## Jak pomóc

> [!IMPORTANT]
> Najlepiej pomożesz, jeśli pobierzesz wersję ze strony [Releases](https://github.com/jub0t/Concat/releases) i zaczniesz z niej korzystać: znajdź miejsca, w których się psuje, i powiedz, co mogłoby działać lepiej.
>
> Chcesz pisać kod? [CONTRIBUTING.md](../CONTRIBUTING.md) opisuje przygotowanie środowiska, strukturę repozytorium, testy do uruchomienia i licencjonowanie wkładu. Sterujesz Concat ze skryptu, usługi albo agenta? [Dokumentacja dla deweloperów](https://concatenate.pages.dev/docs) opisuje API Concat i jego transporty: JSON-RPC i gRPC. W [tej dyskusji](https://github.com/jub0t/Concat/discussions/3) projekt został ogłoszony.
> 
> Współtwórcy mogą dostać rolę `@Contributor` na serwerze Discord: wystarczy poprosić.

## Historia gwiazdek

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
</a>

## Współtwórcy

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## DMCA

Jeśli uważasz, że coś w tym repozytorium, w jego wydaniach albo w rozpowszechnianym przez nie pakiecie lub modelu narusza twoje prawa autorskie, napisz na [jub0trd@gmail.com](mailto:jub0trd@gmail.com) i podaj: opis utworu, do którego masz prawa, link do miejsca, w którym znajduje się tu naruszający materiał, swoje dane kontaktowe, oświadczenie, że w dobrej wierze uważasz, iż takie użycie nie jest dozwolone ani przez ciebie, ani przez twojego pełnomocnika, ani przez prawo, oraz złożone pod rygorem odpowiedzialności za krzywoprzysięstwo oświadczenie, że zgłoszenie jest prawdziwe i że jesteś właścicielem praw lub masz upoważnienie do działania w jego imieniu, opatrzone twoim własnoręcznym lub elektronicznym podpisem. Prawidłowe zgłoszenia są rozpatrywane niezwłocznie, a każdy, czyj materiał został usunięty, może wysłać kontrzgłoszenie (counter-notice) na ten sam adres.
