<p align="center">
  <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark-dark.png" alt="Concat" width="100" />
</p>

<h1 align="center">Concat</h1>
<h3 align="center">Pulsuz, açıq mənbəli və çoxplatformalı CapCut əvəzedicisi.</h3>

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

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇳🇱 Nederlands](README.nl.md) · [🇵🇱 Polski](README.pl.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · **🇦🇿 Azərbaycanca** · [🇭🇷 Hrvatski](README.hr.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇹🇭 ไทย](README.th.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md) · [🇧🇩 বাংলা](README.bn.md)

> İngiliscə README-dən 18 oktyabr 2026-cı ildə tərcümə edilib. Aktual saxlanılan versiya ingiliscə olandır.

## Ödənişli sponsorlar

<table width="100%">
  <tr>
    <td align="center">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>Developerlər üçün istifadə etdikcə ödə proxy-lər<br /><code>JUB0T</code> kodu ilə 20% endirim</sub>
    </td>
  </tr>
</table>

</div>


## Haqqında

Concat macOS, Windows, Linux və Android üçün pulsuz, açıq mənbəli video redaktoru və CapCut alternatividir. İnsanların CapCut-ı əslində nə üçün açdığını qarşılayır: avtomatik subtitrlər, mətndən nitqə, fonun silinməsi, açar kadr animasiyası, effektlər və yazılar, çoxtrekli montaj, 4K ixrac. Heç bir tələ olmadan: su nişanı yoxdur, hesab yoxdur, abunəlik yoxdur, yükləmə yoxdur.

Hər şey GPU kompozitorlu doğma Rust mühərrikində lokal işləyir. Quraşdırın, videonu atın, kəsin. Subtitrlər, səslər və fon kəsimi üçün süni intellekt modelləri Parametrlərdən bir dəfə yüklənir və ondan sonra oflayn işləyir. Videolarınız heç vaxt diskinizdən çıxmır.

**Nə üçün uyğundur:** TikTok, Reels və Shorts, YouTube videoları, dərslər və ekran yazıları, podkast kəsikləri, memlar.

**Maşınlar üçün də:** süni intellekt agentləri üçün daxili MCP serveri (`concat --mcp`, [quraşdırma](MCP.md)) və skriptlər üçün JSON-RPC API, beləliklə onlar da onunla video kəsə bilər.

## Əsas xüsusiyyətlər

- 🚫 **Su nişanı yox. Hesab yox. Ödəniş divarı yox.** Heç vaxt.
- 🔒 **100% lokal.** Heç nə yüklənmir. Oflayn işləyir.
- 💬 **Avtomatik subtitrlər.** Lokal Whisper. Model ölçüsünü seçin, zaman xəttində stilləşdirilmiş subtitrlər alın.
- 🗣️ **Mətndən nitqə + səs klonlama.** Pulsuz lokal səslər, ya da bir neçə saniyəlik yazıdan istənilən səs.
- 🧍 **Fonun silinməsi.** İnsanlar, obyektlər, ya da maskanı özünüz çəkin.
- 🎞️ **Açar kadrlar.** Mövqe, miqyas, fırlanma, şəffaflıq, səs səviyyəsi, effekt parametrləri. Daxili əyri redaktoru.
- ✨ **100 effekt, filtr və keçid.** GPU ilə render olunur, önizləmədə canlı.
- ✂️ **Sürətli kəsin.** Bölmə, kəsib qısaltma, ripple, birləşdirmə, kadrı dondurma, sürət. İstəsəniz maqnit zaman xətti.
- 🎚️ **Çoxtrekli, çox zaman xətli.** Bir layihədə bir neçə montaj. Qarışdırma rejimləri, kəsmə, çevirmələr.
- 📝 **Yazılar.** Şriftlər, kontur, kölgə, fon lövhəsi. Başlamaq üçün hazır şablonlar.
- 🎙️ **Bir düymə ilə səs təmizləmə.** Küyün azaldılması, səsin yaxşılaşdırılması, səs gücünün tarazlanması. Üstəlik sincab, robot, telefon və digərləri.
- 📤 **İxrac.** H.264, HEVC, AV1. 4K 60-a qədər, 10-bit rəng.
- 🦀 **Doğma Rust mühərriki.** GPU kompozitoru, proxy-lər, aparat dekodlaması. 4K-da hamar gəzinti.
- 🤖 **Skriptlə idarə oluna bilər.** Daxili MCP serveri sayəsində Claude və digər süni intellekt agentləri onunla montaj edə bilər ([bir sətirlə quraşdırma](MCP.md)), skriptlər üçün isə JSON-RPC API var. Mənbədən yığılan versiya buna gRPC və komanda sətri aləti əlavə edir.
- 🖥️ **macOS, Windows, Linux, Android.** 26 dil. Eyni tətbiq, eyni layihə faylları.

## Yüklə

İki yol var:

1. **[Veb sayt](https://concatenate.pages.dev/#download)** cihazınız üçün düzgün versiyanı verir. Buradan başlayın.
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** hər platforma üçün bütün versiyaları quraşdırıcılar, paketlər və yoxlama cəmləri ilə təqdim edir. Özünüz seçmək istədikdə.

Concat **beta** mərhələsindədir: işləyir, amma hələ də kələ-kötür yerləri var. Birini tapanda [bildirin](https://github.com/jub0t/Concat/issues).

**Platformalar**

- ✅ **Windows** · x86_64 (ARM versiyaları dayandırılıb, bax: [#326](https://github.com/jub0t/Concat/issues/326)). Quraşdırıcı və `.msi`. SmartScreen imzasız versiyanı dayandırsa: **More info** › **Run anyway**
- ✅ **macOS** · Intel və Apple silicon. macOS imzasız versiyanı açmaqdan imtina etsə: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 və ARM. `.deb`, `.rpm`, `.AppImage` və Arch paketi. Həmçinin **[Flatpark](https://flatpark.org/apps/app.concat.editor/)** üzərində Flatpak: hər buraxılışın x86_64 `.deb` paketini bükən və onunla birgə yenilənən icma Flatpak deposu
- ✅ **Android** · telefonlar və planşetlər
- ✅ **iOS / iPadOS** · iPhone və iPad, sideload ilə

✅ Dəstəklənir · 🚧 Üzərində işlənir · 🧪 Test olunacaq

**Sistem tələbləri** və əlavə model ölçüləri [veb saytdadır](https://concatenate.pages.dev/guides/system-requirements).

**[Code signing policy](../CODE_SIGNING.md)**

## Necə töhfə vermək olar

> [!IMPORTANT]
> Töhfə verməyin ən yaxşı yolu [Releases](https://github.com/jub0t/Concat/releases) səhifəsindən bir versiya götürüb istifadə etməkdir: harada sındığını tapın və harada daha yaxşı ola biləcəyini deyin.
>
> Kod yazmağa hazırsınız? [CONTRIBUTING.md](../CONTRIBUTING.md) quraşdırmanı, ağacın strukturunu, işə salınacaq yoxlamaları və töhfələrin necə lisenziyalaşdırıldığını izah edir. Concat-ı skriptdən, servisdən və ya agentdən idarə edirsiniz? [Developer sənədləri](https://concatenate.pages.dev/docs) Concat API-ni və onun nəqliyyat protokollarını izah edir: JSON-RPC və gRPC; [docs/MCP.md](MCP.md) süni intellekt agentini MCP üzərindən qoşur. Layihə [bu müzakirədə](https://github.com/jub0t/Concat/discussions/3) elan edilib.
> 
> Töhfə verənlər Discord serverində `@Contributor` rolu ala bilər: sadəcə soruşun.

## Ulduz tarixçəsi

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
</a>

## Töhfə verənlər

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## DMCA

Bu depodakı, onun buraxılışlarındakı və ya yaydığı paket ya da modeldəki nəyinsə müəllif hüququnuzu pozduğunu düşünürsünüzsə, [jub0trd@gmail.com](mailto:jub0trd@gmail.com) ünvanına aşağıdakıları ehtiva edən e-poçt göndərin: sahibi olduğunuz əsərin təsviri, pozuntu yaradan materialın burada göründüyü yerin linki, əlaqə məlumatlarınız, istifadəyə sizin, nümayəndənizin və ya qanunun icazə vermədiyinə vicdanla inandığınızı bildirən bəyanat, və yalan ifadəyə görə məsuliyyəti öz üzərinizə götürərək bildirişin doğru olduğunu və hüquq sahibi olduğunuzu ya da onun adından hərəkət etməyə səlahiyyətli olduğunuzu bildirən, fiziki və ya elektron imzanızla təsdiqlənmiş bəyanat. Etibarlı bildirişlər tez bir zamanda icra olunur, materialı silinən hər kəs isə eyni ünvana əks-bildiriş göndərə bilər.
