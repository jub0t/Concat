<p align="center">
  <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark-dark.png" alt="Concat" width="100" />
</p>

<h1 align="center">Concat</h1>
<h3 align="center">CapCut का मुफ़्त, ओपन-सोर्स और क्रॉस-प्लेटफ़ॉर्म विकल्प।</h3>

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

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇳🇱 Nederlands](README.nl.md) · [🇵🇱 Polski](README.pl.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇦🇿 Azərbaycanca](README.az.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇹🇭 ไทย](README.th.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · **🇮🇳 हिन्दी** · [🇵🇰 اردو](README.ur.md) · [🇧🇩 বাংলা](README.bn.md)

> 11 अक्टूबर 2026 को अंग्रेज़ी README से अनुवादित। अंग्रेज़ी संस्करण ही अद्यतन रखा जाता है।

## सशुल्क प्रायोजक

<table width="100%">
  <tr>
    <td align="center">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>डेवलपर्स के लिए उपयोग-अनुसार भुगतान वाले प्रॉक्सी<br />कोड <code>JUB0T</code> से 20 % छूट</sub>
    </td>
  </tr>
</table>

</div>


## परिचय

Concat macOS, Windows, Linux और Android के लिए एक मुफ़्त, ओपन-सोर्स वीडियो एडिटर और CapCut का विकल्प है। यह वही सब करता है जिसके लिए लोग असल में CapCut खोलते हैं: ऑटो-कैप्शन, टेक्स्ट-टू-स्पीच, बैकग्राउंड हटाना, कीफ़्रेम एनिमेशन, इफ़ेक्ट और टाइटल, मल्टी-ट्रैक कटिंग, 4K एक्सपोर्ट। और किसी शर्त के बिना: न वॉटरमार्क, न अकाउंट, न सब्सक्रिप्शन, न अपलोड।

सब कुछ GPU कंपोज़िटर वाले नेटिव Rust इंजन पर लोकली चलता है। इंस्टॉल करें, फ़ुटेज डालें, काटें। कैप्शन, आवाज़ और कटआउट के AI मॉडल सेटिंग्स से एक बार डाउनलोड होते हैं और उसके बाद ऑफ़लाइन काम करते हैं। आपकी फ़ुटेज कभी आपकी डिस्क से बाहर नहीं जाती।

**इनके लिए अच्छा:** TikTok, Reels और Shorts, YouTube वीडियो, ट्यूटोरियल और स्क्रीन रिकॉर्डिंग, पॉडकास्ट क्लिप, मीम।

**मशीनों के लिए भी:** एक JSON-RPC API, ताकि स्क्रिप्ट और AI एजेंट भी इससे वीडियो काट सकें।

## मुख्य बातें

- 🚫 **न वॉटरमार्क। न अकाउंट। न पेवॉल।** कभी नहीं।
- 🔒 **100 % लोकल।** कुछ भी अपलोड नहीं होता। ऑफ़लाइन काम करता है।
- 💬 **ऑटो-कैप्शन।** लोकल Whisper। मॉडल का आकार चुनें, टाइमलाइन पर स्टाइल किए कैप्शन पाएँ।
- 🗣️ **टेक्स्ट-टू-स्पीच + वॉइस क्लोनिंग।** मुफ़्त लोकल आवाज़ें, या कुछ सेकंड की रिकॉर्डिंग से कोई भी आवाज़।
- 🧍 **बैकग्राउंड हटाना।** लोग, वस्तुएँ, या मास्क खुद पेंट करें।
- 🎞️ **कीफ़्रेम।** स्थिति, स्केल, घुमाव, अपारदर्शिता, वॉल्यूम, इफ़ेक्ट पैरामीटर। कर्व एडिटर बिल्ट-इन।
- ✨ **100 इफ़ेक्ट, फ़िल्टर और ट्रांज़िशन।** GPU से रेंडर, प्रीव्यू में लाइव।
- ✂️ **तेज़ी से काटें।** स्प्लिट, ट्रिम, रिपल, मर्ज, फ़्रीज़ फ़्रेम, स्पीड। चाहें तो मैग्नेटिक टाइमलाइन।
- 🎚️ **मल्टी-ट्रैक, मल्टी-टाइमलाइन।** एक प्रोजेक्ट में कई कट। ब्लेंड मोड, क्रॉप, फ़्लिप।
- 📝 **टाइटल।** फ़ॉन्ट, स्ट्रोक, शैडो, बैकग्राउंड प्लेट। शुरू करने के लिए प्रीसेट।
- 🎙️ **एक स्विच से आवाज़ की सफ़ाई।** शोर हटाएँ, आवाज़ निखारें, लाउडनेस बराबर करें। साथ में चिपमंक, रोबोट, टेलीफ़ोन वगैरह।
- 📤 **एक्सपोर्ट।** H.264, HEVC, AV1। 4K 60 तक, 10-बिट रंग।
- 🦀 **नेटिव Rust इंजन।** GPU कंपोज़िटर, प्रॉक्सी, हार्डवेयर डिकोड। 4K आसानी से स्क्रब होता है।
- 🤖 **स्क्रिप्ट किया जा सकता है।** एक JSON-RPC API, जिससे स्क्रिप्ट और AI एजेंट वीडियो काट सकते हैं। सोर्स से बिल्ड करने पर gRPC और एक कमांड-लाइन टूल भी मिलता है।
- 🖥️ **macOS, Windows, Linux, Android।** 26 भाषाएँ। वही ऐप, वही प्रोजेक्ट फ़ाइलें।

## डाउनलोड

दो रास्ते:

1. **[वेबसाइट](https://concatenate.pages.dev/#download)** आपकी मशीन के लिए सही बिल्ड देती है। यहीं से शुरू करें।
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** पर हर प्लेटफ़ॉर्म की हर बिल्ड है, इंस्टॉलर, पैकेज और चेकसम के साथ। जब आप खुद चुनना चाहें।

Concat **बीटा** में है: यह काम करता है, और इसमें अभी कुछ खुरदरापन है। कोई मिले तो [बताइए](https://github.com/jub0t/Concat/issues)।

**प्लेटफ़ॉर्म**

- ✅ **Windows** · x86_64 (ARM बिल्ड फ़िलहाल रोके गए हैं, देखें [#326](https://github.com/jub0t/Concat/issues/326))। एक सेटअप और एक `.msi`। अगर SmartScreen किसी बिना हस्ताक्षर वाली बिल्ड को रोके: **More info** › **Run anyway**
- ✅ **macOS** · Intel और Apple Silicon। अगर macOS बिना हस्ताक्षर वाली बिल्ड खोलने से मना करे: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 और ARM। `.deb`, `.rpm`, `.AppImage` और एक Arch पैकेज। साथ ही **[Flatpark](https://flatpark.org/apps/app.concat.editor/)** पर एक Flatpak, जो एक सामुदायिक Flatpak रिमोट है और हर रिलीज़ की x86_64 `.deb` को लपेटकर उसके साथ अपडेट होता है
- ✅ **Android** · फ़ोन और टैबलेट
- ✅ **iOS / iPadOS** · iPhone और iPad, साइडलोड करके

✅ समर्थित · 🚧 काम जारी · 🧪 परखना बाकी

**सिस्टम आवश्यकताएँ** और वैकल्पिक मॉडलों के आकार [वेबसाइट](https://concatenate.pages.dev/guides/system-requirements) पर हैं।

**[Code signing policy](../CODE_SIGNING.md)**

## योगदान कैसे करें

> [!IMPORTANT]
> योगदान का सबसे अच्छा तरीका है [Releases](https://github.com/jub0t/Concat/releases) पेज से एक बिल्ड लेकर उसे इस्तेमाल करना: पता लगाएँ कहाँ टूटता है, और बताएँ कहाँ बेहतर हो सकता है।
>
> कोड लिखने को तैयार हैं? [CONTRIBUTING.md](../CONTRIBUTING.md) में सेटअप, ट्री की बनावट, चलाई जाने वाली जाँचें और योगदान के लाइसेंस की जानकारी है। किसी स्क्रिप्ट, सर्विस या एजेंट से Concat चला रहे हैं? [डेवलपर डॉक्स](https://concatenate.pages.dev/docs) में Concat API और उसके ट्रांसपोर्ट हैं: JSON-RPC और gRPC। प्रोजेक्ट की घोषणा [इस Discussion](https://github.com/jub0t/Concat/discussions/3) में हुई थी।
> 
> योगदानकर्ता Discord सर्वर पर `@Contributor` रोल ले सकते हैं: बस माँग लें।

## स्टार इतिहास

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
</a>

## योगदानकर्ता

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## DMCA

अगर आपको लगता है कि इस रिपॉज़िटरी, इसकी रिलीज़, या इसके द्वारा वितरित किसी पैकेज या मॉडल में कुछ आपके कॉपीराइट का उल्लंघन करता है, तो [jub0trd@gmail.com](mailto:jub0trd@gmail.com) पर ईमेल करें और उसमें ये शामिल करें: आपके स्वामित्व वाली कृति का विवरण, उस जगह का लिंक जहाँ उल्लंघन करने वाली सामग्री यहाँ दिखती है, आपकी संपर्क जानकारी, यह कथन कि आप सद्भावपूर्वक मानते हैं कि इस उपयोग की अनुमति न आपने, न आपके एजेंट ने और न ही क़ानून ने दी है, और झूठी गवाही के दंड के अधीन यह कथन कि नोटिस सटीक है और आप स्वामी हैं या स्वामी की ओर से कार्य करने के लिए अधिकृत हैं, जिस पर आपके हस्तलिखित या इलेक्ट्रॉनिक हस्ताक्षर हों। वैध नोटिसों पर तुरंत कार्रवाई की जाती है, और जिस किसी की सामग्री हटाई जाती है, वह उसी पते पर काउंटर-नोटिस भेज सकता है।
