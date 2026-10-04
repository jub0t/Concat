<div align="center">
<table width="100%">
  <tr>
    <td align="left" width="120">
      <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark.png" alt="Concat" width="100" />
    </td>
    <td align="">
      <h1>Concat</h1>
      <h3 style="margin-top: -10px;">CapCut का सचमुच मुफ़्त, ओपन-सोर्स और क्रॉस-प्लेटफ़ॉर्म विकल्प।</h3>
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

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · **🇮🇳 हिन्दी** · [🇵🇰 اردو](README.ur.md)

> 2 अक्टूबर 2026 को अंग्रेज़ी README से अनुवादित। अंग्रेज़ी संस्करण ही अद्यतन रखा जाता है।

## सशुल्क प्रायोजक

<table width="100%">
  <tr>
    <td align="center" width="50%">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>डेवलपर्स के लिए उपयोग-अनुसार भुगतान वाले प्रॉक्सी<br />कोड <code>JUB0T</code> से 20 % छूट</sub>
    </td>
    <td align="center" width="50%">
      <a href="#sponsoring"><b>आपका लोगो यहाँ</b></a><br />
      <sub><a href="#sponsoring">Concat को प्रायोजित करें</a> और आपका नाम, लोगो और लिंक इस जगह आ जाएगा</sub>
    </td>
  </tr>
</table>

</div>


## परिचय

Concat macOS, Windows, Linux और Android के लिए एक मुफ़्त, ओपन-सोर्स वीडियो एडिटर और CapCut का विकल्प है। यह वही सब करता है जिसके लिए लोग असल में CapCut खोलते हैं: ऑटो-कैप्शन, टेक्स्ट-टू-स्पीच, बैकग्राउंड हटाना, कीफ़्रेम एनिमेशन, इफ़ेक्ट और टाइटल, मल्टी-ट्रैक कटिंग, 4K एक्सपोर्ट। और किसी शर्त के बिना: न वॉटरमार्क, न अकाउंट, न सब्सक्रिप्शन, न अपलोड।

सब कुछ GPU कंपोज़िटर वाले नेटिव Rust इंजन पर लोकली चलता है। इंस्टॉल करें, फ़ुटेज डालें, काटें। कैप्शन, आवाज़ और कटआउट के AI मॉडल सेटिंग्स से एक बार डाउनलोड होते हैं और उसके बाद ऑफ़लाइन काम करते हैं। आपकी फ़ुटेज कभी आपकी डिस्क से बाहर नहीं जाती।

**इनके लिए अच्छा:** TikTok, Reels और Shorts, YouTube वीडियो, ट्यूटोरियल और स्क्रीन रिकॉर्डिंग, पॉडकास्ट क्लिप, मीम।

**मशीनों के लिए भी:** एक JSON-RPC, gRPC और MCP API, ताकि स्क्रिप्ट और AI एजेंट भी इससे वीडियो काट सकें।

## मुख्य बातें

- 🚫 **न वॉटरमार्क। न अकाउंट। न पेवॉल।** कभी नहीं।
- 🔒 **100 % लोकल।** कुछ भी अपलोड नहीं होता। ऑफ़लाइन काम करता है।
- 💬 **ऑटो-कैप्शन।** लोकल Whisper। मॉडल का आकार चुनें, टाइमलाइन पर स्टाइल किए कैप्शन पाएँ।
- 🗣️ **टेक्स्ट-टू-स्पीच + वॉइस क्लोनिंग।** मुफ़्त लोकल आवाज़ें, या कुछ सेकंड की रिकॉर्डिंग से कोई भी आवाज़।
- 🧍 **बैकग्राउंड हटाना।** लोग, वस्तुएँ, या मास्क खुद पेंट करें।
- 🎞️ **कीफ़्रेम।** स्थिति, स्केल, घुमाव, अपारदर्शिता, वॉल्यूम, इफ़ेक्ट पैरामीटर। कर्व एडिटर बिल्ट-इन।
- ✨ **170+ इफ़ेक्ट, फ़िल्टर, ट्रांज़िशन और टेक्स्ट एनिमेशन।** GPU से रेंडर, प्रीव्यू में लाइव।
- ✂️ **तेज़ी से काटें।** स्प्लिट, ट्रिम, रिपल, मर्ज, फ़्रीज़ फ़्रेम, स्पीड। चाहें तो मैग्नेटिक टाइमलाइन।
- 🎚️ **मल्टी-ट्रैक, मल्टी-टाइमलाइन।** एक प्रोजेक्ट में कई कट। ब्लेंड मोड, क्रॉप, फ़्लिप।
- 📝 **टाइटल।** फ़ॉन्ट, स्ट्रोक, शैडो, बैकग्राउंड प्लेट। शुरू करने के लिए प्रीसेट।
- 🎙️ **एक स्विच से आवाज़ की सफ़ाई।** शोर हटाएँ, आवाज़ निखारें, लाउडनेस बराबर करें। साथ में चिपमंक, रोबोट, टेलीफ़ोन वगैरह।
- 📤 **एक्सपोर्ट।** H.264, HEVC, AV1। 4K 60 तक, 10-बिट रंग।
- 🦀 **नेटिव Rust इंजन।** GPU कंपोज़िटर, प्रॉक्सी, हार्डवेयर डिकोड। 4K आसानी से स्क्रब होता है।
- 🤖 **स्क्रिप्ट किया जा सकता है।** JSON-RPC, gRPC और MCP API, साथ में एक CLI। AI एजेंट इससे वीडियो काट सकते हैं।
- 🖥️ **macOS, Windows, Linux, Android।** 14 भाषाएँ। वही ऐप, वही प्रोजेक्ट फ़ाइलें।

## डाउनलोड

दो रास्ते:

1. **[वेबसाइट](https://concatenate.pages.dev/#download)** आपकी मशीन के लिए सही बिल्ड देती है। यहीं से शुरू करें।
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** पर हर प्लेटफ़ॉर्म की हर बिल्ड है, इंस्टॉलर, पैकेज और चेकसम के साथ। जब आप खुद चुनना चाहें।

Concat **बीटा** में है: यह काम करता है, और इसमें अभी कुछ खुरदरापन है। कोई मिले तो [बताइए](https://github.com/jub0t/Concat/issues)।

**प्लेटफ़ॉर्म**

- ✅ **Windows** · x86_64 और ARM। एक सेटअप और एक `.msi`। अगर SmartScreen किसी बिना हस्ताक्षर वाली बिल्ड को रोके: **More info** › **Run anyway**
- ✅ **macOS** · Intel और Apple Silicon। अगर macOS बिना हस्ताक्षर वाली बिल्ड खोलने से मना करे: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 और ARM। `.deb`, `.rpm`, `.AppImage` और एक Arch पैकेज। साथ ही **[Flatpark](https://flatpark.org/apps/app.concat.editor/)** पर एक Flatpak, जो एक सामुदायिक Flatpak रिमोट है और हर रिलीज़ की x86_64 `.deb` को लपेटकर उसके साथ अपडेट होता है
- ✅ **Android** · फ़ोन और टैबलेट
- ✅ **iOS / iPadOS** · iPhone और iPad, साइडलोड करके

✅ समर्थित · 🚧 काम जारी · 🧪 परखना बाकी

**सिस्टम आवश्यकताएँ** और वैकल्पिक मॉडलों के आकार [वेबसाइट](https://concatenate.pages.dev/guides/system-requirements) पर हैं।

## शुरू करें

डाउनलोड करें, खोलें, फ़ुटेज डालें, काटें। न अकाउंट, न सेटअप।

**कुछ रिपोर्ट करना हो तो:** हर रन एक लॉग लिखता है, और सेटिंग्स › About में उसे खोलने का बटन है, उसी के बगल में आपके सिस्टम की जानकारी कॉपी करने वाला बटन। दोनों को एक [issue](https://github.com/jub0t/Concat/issues) में जोड़ दें, रिपोर्ट ज़रूरी सब कुछ लेकर पहुँच जाती है। पिछले दस रन रखे जाते हैं, तो कल वाला भी अभी है; कुछ भी अपने आप कहीं नहीं भेजा जाता।

## योगदान कैसे करें

> [!IMPORTANT]
> योगदान का सबसे अच्छा तरीका है [Releases](https://github.com/jub0t/Concat/releases) पेज से एक बिल्ड लेकर उसे इस्तेमाल करना: पता लगाएँ कहाँ टूटता है, और बताएँ कहाँ बेहतर हो सकता है।
>
> कोड लिखने को तैयार हैं? [CONTRIBUTING.md](../CONTRIBUTING.md) में सेटअप, ट्री की बनावट, चलाई जाने वाली जाँचें और योगदान के लाइसेंस की जानकारी है। किसी स्क्रिप्ट, सर्विस या एजेंट से Concat चला रहे हैं? [डेवलपर डॉक्स](https://concatenate.pages.dev/docs) में Concat API और उसके ट्रांसपोर्ट हैं: JSON-RPC, gRPC और MCP। प्रोजेक्ट की घोषणा [इस Discussion](https://github.com/jub0t/Concat/discussions/3) में हुई थी।
> 
> योगदानकर्ता Discord सर्वर पर `@Contributor` रोल ले सकते हैं, बस माँग लें।

## योगदानकर्ता

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## स्टार इतिहास

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&theme=dark&legend=bottom-right" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
 </picture>
</a>

<a id="sponsoring"></a>
## प्रायोजन

Concat में कोई पेवॉल नहीं है और कभी नहीं होगा: न वॉटरमार्क, न अकाउंट, न सशुल्क टियर। उसकी जगह प्रायोजन लेता है। अगर Concat ने आपके लिए किसी सब्सक्रिप्शन की जगह ले ली है, तो उसका एक अंश इसे चलाए रखता है।

**पैसा कहाँ जाता है**

[रोडमैप](https://concatenate.pages.dev/roadmap) बताता है कि प्रायोजन से क्या-क्या बनता है और हर हिस्से की लागत कितनी है।

[वेबसाइट](https://concatenate.pages.dev/#sponsor) पर एक टियर चुनें।

<a href="https://concatenate.pages.dev/#sponsor"><img src="https://img.shields.io/badge/Sponsor-Concat-0568FD?style=flat-square&labelColor=212123" alt="Sponsor Concat on the website" /></a>

अभी योगदान देने की स्थिति में नहीं हैं? एक स्टार, एक बग रिपोर्ट, या वीडियो एडिट करने वाले किसी को एक बात बताना भी बहुत मायने रखता है।
