<div align="center">
<table width="100%">
  <tr>
    <td align="left" width="120">
      <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark.png" alt="Concat" width="100" />
    </td>
    <td align="">
      <h1>Concat</h1>
      <h3 style="margin-top: -10px;">Gerçekten ücretsiz, açık kaynaklı ve çok platformlu CapCut alternatifi.</h3>
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

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · **🇹🇷 Türkçe** · [🇭🇷 Hrvatski](README.hr.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md)

> İngilizce README’den 2 Ekim 2026’da çevrildi. Güncel tutulan sürüm İngilizce olandır.

## Ücretli Sponsorlar

<table width="100%">
  <tr>
    <td align="center" width="50%">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>Geliştiriciler için kullandıkça öde proxy’ler<br /><code>JUB0T</code> koduyla %20 indirim</sub>
    </td>
    <td align="center" width="50%">
      <a href="#sponsoring"><b>Logonuz burada</b></a><br />
      <sub><a href="#sponsoring">Concat’e sponsor olun</a> ve adınız, logonuz ve bağlantınız bu yeri alsın</sub>
    </td>
  </tr>
</table>

</div>


## Hakkında

Concat; macOS, Windows, Linux ve Android için ücretsiz, açık kaynaklı bir video düzenleyici ve bir CapCut alternatifidir. İnsanların CapCut’ı gerçekten ne için açtıklarını karşılar: otomatik altyazı, metinden sese, arka plan kaldırma, anahtar kare animasyonu, efektler ve yazılar, çok parçalı kurgu, 4K dışa aktarma. Hiçbir tuzak olmadan: filigran yok, hesap yok, abonelik yok, yükleme yok.

Her şey, GPU birleştiricili yerel bir Rust motorunda yerel olarak çalışır. Kurun, görüntüleri bırakın, kesin. Altyazı, ses ve kesme için yapay zekâ modelleri Ayarlar’dan bir kez indirilir ve sonrasında çevrimdışı çalışır. Görüntüleriniz diskinizden asla çıkmaz.

**Şunlar için uygun:** TikTok, Reels ve Shorts, YouTube videoları, eğitimler ve ekran kayıtları, podcast kesitleri, memler.

**Makineler için de:** bir JSON-RPC, gRPC ve MCP API’si; böylece betikler ve yapay zekâ ajanları da onunla video kesebilir.

## Öne çıkanlar

- 🚫 **Filigran yok. Hesap yok. Ödeme duvarı yok.** Asla.
- 🔒 **%100 yerel.** Hiçbir şey yüklenmez. Çevrimdışı çalışır.
- 💬 **Otomatik altyazı.** Yerel Whisper. Bir model boyutu seçin, zaman çizelgesinde biçimli altyazılar alın.
- 🗣️ **Metinden sese + ses klonlama.** Ücretsiz yerel sesler ya da birkaç saniyelik kayıttan herhangi bir ses.
- 🧍 **Arka plan kaldırma.** İnsanlar, nesneler ya da maskeyi kendiniz boyayın.
- 🎞️ **Anahtar kareler.** Konum, ölçek, döndürme, saydamlık, ses düzeyi, efekt parametreleri. Yerleşik eğri düzenleyici.
- ✨ **170’ten fazla efekt, filtre, geçiş ve yazı animasyonu.** GPU ile çizilir, önizlemede canlı.
- ✂️ **Hızlı kesin.** Böl, kırp, ripple, birleştir, kareyi dondur, hız. İsterseniz manyetik zaman çizelgesi.
- 🎚️ **Çok parçalı, çok zaman çizelgeli.** Bir projede birden çok kurgu. Karışım modları, kırpma, çevirmeler.
- 📝 **Yazılar.** Yazı tipleri, kontur, gölge, arka plan plakası. Başlamak için hazır ayarlar.
- 🎙️ **Tek anahtarla ses temizliği.** Gürültü azaltma, sesi iyileştirme, ses düzeyini dengeleme. Artı sincap, robot, telefon ve arkadaşları.
- 📤 **Dışa aktarma.** H.264, HEVC, AV1. 4K 60’a kadar, 10 bit renk.
- 🦀 **Yerel Rust motoru.** GPU birleştirici, proxy’ler, donanım çözme. 4K akıcı gezinir.
- 🤖 **Betiklenebilir.** JSON-RPC, gRPC ve MCP API’si, artı bir CLI. Yapay zekâ ajanları onunla video kesebilir.
- 🖥️ **macOS, Windows, Linux, Android.** 14 dil. Aynı uygulama, aynı proje dosyaları.

## İndir

İki yol var:

1. **[Web sitesi](https://concatenate.pages.dev/#download)** makineniz için doğru derlemeyi verir. Buradan başlayın.
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** her platform için her derlemeyi kurucular, paketler ve sağlama toplamlarıyla sunar. Kendiniz seçmek istediğinizde.

Concat **beta** aşamasında: çalışıyor, ama hâlâ pürüzleri var. Bir tane bulduğunuzda [söyleyin](https://github.com/jub0t/Concat/issues).

**Platformlar**

- ✅ **Windows** · x86_64 ve ARM. Bir kurulum ve bir `.msi`. SmartScreen imzasız bir derlemeyi durdurursa: **Daha fazla bilgi** › **Yine de çalıştır**
- ✅ **macOS** · Intel ve Apple Silicon. macOS imzasız bir derlemeyi açmayı reddederse: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 ve ARM. `.deb`, `.rpm`, `.AppImage` ve bir Arch paketi. Ayrıca her sürümün x86_64 `.deb`’ini saran ve onunla güncellenen topluluk Flatpak deposu **[Flatpark](https://flatpark.org/apps/app.concat.editor/)** üzerinde bir Flatpak
- ✅ **Android** · telefonlar ve tabletler
- ✅ **iOS / iPadOS** · iPhone ve iPad, sideload ile

✅ Destekleniyor · 🚧 Çalışılıyor · 🧪 Test edilecek

**Sistem gereksinimleri** ve isteğe bağlı model boyutları [web sitesinde](https://concatenate.pages.dev/guides/system-requirements).

## Başlarken

İndirin, açın, görüntüleri bırakın, kesin. Hesap yok, kurulum yok.

**Bir şey bildirmek için:** her çalıştırma bir günlük yazar ve Ayarlar › Hakkında’da onu açan düğme, sistem bilgilerinizi kopyalayan düğmenin yanındadır. İkisini de bir [issue](https://github.com/jub0t/Concat/issues)’a ekleyin; rapor ihtiyacı olan her şeyle gelir. Son on çalıştırma saklanır, yani dünkü hâlâ orada; hiçbir şey kendiliğinden hiçbir yere gönderilmez.

## Nasıl katkıda bulunulur

> [!IMPORTANT]
> Katkıda bulunmanın en iyi yolu [Releases](https://github.com/jub0t/Concat/releases) sayfasından bir derleme alıp kullanmaktır: nerede bozulduğunu bulun ve nerede daha iyi olabileceğini söyleyin.
>
> Kod yazmaya hazır mısınız? [CONTRIBUTING.md](../CONTRIBUTING.md) kurulumu, ağacın düzenini, çalıştırılacak denetimleri ve katkıların nasıl lisanslandığını anlatır. Concat’i bir betikten, servisten ya da ajandan mı sürüyorsunuz? [Geliştirici belgeleri](https://concatenate.pages.dev/docs) Concat API’sini ve taşıyıcılarını anlatır: JSON-RPC, gRPC ve MCP. Proje [bu tartışmada](https://github.com/jub0t/Concat/discussions/3) duyuruldu.
> 
> Katkıda bulunanlar Discord sunucusunda bir `@Contributor` rolü alabilir, istemeniz yeter.

## Katkıda bulunanlar

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## Yıldız geçmişi

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&theme=dark&legend=bottom-right" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
 </picture>
</a>

<a id="sponsoring"></a>
## Sponsorluk

Concat’in ödeme duvarı yok ve asla olmayacak: filigran yok, hesap yok, ücretli katman yok. Sponsorluk onun yerini tutar. Concat sizin için bir aboneliğin yerine geçtiyse, onun küçük bir kısmı Concat’i ayakta tutar.

**Nereye gidiyor**

[Yol haritası](https://concatenate.pages.dev/roadmap) sponsorluğun neyi karşıladığını ve her parçanın ne kadara mal olduğunu anlatır.

[Web sitesinde](https://concatenate.pages.dev/#sponsor) bir katman seçin.

<a href="https://concatenate.pages.dev/#sponsor"><img src="https://img.shields.io/badge/Sponsor-Concat-0568FD?style=flat-square&labelColor=212123" alt="Sponsor Concat on the website" /></a>

Katkıda bulunacak durumda değil misiniz? Bir yıldız, bir hata bildirimi ya da video düzenleyen birine bir söz de çok şey ifade eder.
