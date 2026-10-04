<div align="center">
<table width="100%">
  <tr>
    <td align="left" width="120">
      <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark.png" alt="Concat" width="100" />
    </td>
    <td align="">
      <h1>Concat</h1>
      <h3 style="margin-top: -10px;">התחליף החינמי באמת, בקוד פתוח וחוצה פלטפורמות ל-CapCut.</h3>
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

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · **🇮🇱 עברית** · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md)

> תורגם מה-README האנגלי ב-2 באוקטובר 2026. הגרסה האנגלית היא זו שמתעדכנת.

## נותני חסות בתשלום

<table width="100%">
  <tr>
    <td align="center" width="50%">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>פרוקסי בתשלום לפי שימוש למפתחים<br />עם הקוד <code>JUB0T</code> מקבלים 20 % הנחה</sub>
    </td>
    <td align="center" width="50%">
      <a href="#sponsoring"><b>הלוגו שלך כאן</b></a><br />
      <sub><a href="#sponsoring">תנו חסות ל-Concat</a> והשם, הלוגו והקישור שלכם יתפסו את המקום הזה</sub>
    </td>
  </tr>
</table>

</div>


## אודות

Concat הוא עורך וידאו חינמי בקוד פתוח וחלופה ל-CapCut עבור macOS, Windows, Linux ו-Android. הוא מכסה את מה שאנשים באמת פותחים בשבילו את CapCut: כתוביות אוטומטיות, טקסט לדיבור, הסרת רקע, אנימציית פריימים, אפקטים וכותרות, עריכה מרובת רצועות, ייצוא ב-4K. ובלי שום מלכודות: בלי סימן מים, בלי חשבון, בלי מנוי, בלי העלאה.

הכול רץ מקומית על מנוע Rust נייטיב עם קומפוזיטור GPU. מתקינים, גוררים חומר, חותכים. מודלי ה-AI לכתוביות, לקולות ולחיתוך רקע יורדים פעם אחת מההגדרות ואחר כך עובדים במצב לא מקוון. החומר שלכם לעולם לא עוזב את הדיסק.

**מתאים ל:** TikTok, Reels ו-Shorts, סרטוני YouTube, מדריכים והקלטות מסך, קטעי פודקאסט, ממים.

**וגם למכונות:** API של JSON-RPC, gRPC ו-MCP, כך שגם סקריפטים וסוכני AI יכולים לערוך איתו וידאו.

## עיקרי הדברים

- 🚫 **בלי סימני מים. בלי חשבון. בלי חומת תשלום.** לעולם.
- 🔒 **100 % מקומי.** שום דבר לא עולה. עובד בלי אינטרנט.
- 💬 **כתוביות אוטומטיות.** Whisper מקומי. בוחרים גודל מודל ומקבלים כתוביות מעוצבות על ציר הזמן.
- 🗣️ **טקסט לדיבור + שיבוט קול.** קולות מקומיים בחינם, או כל קול מתוך כמה שניות של הקלטה.
- 🧍 **הסרת רקע.** אנשים, עצמים, או ציירו את המסכה בעצמכם.
- 🎞️ **פריימים.** מיקום, קנה מידה, סיבוב, אטימות, עוצמה, פרמטרים של אפקטים. עורך עקומות מובנה.
- ✨ **יותר מ-170 אפקטים, פילטרים, מעברים ואנימציות טקסט.** מרונדרים ב-GPU, חיים בתצוגה המקדימה.
- ✂️ **חותכים מהר.** פיצול, קיצוץ, ripple, מיזוג, הקפאת פריים, מהירות. ציר זמן מגנטי אם רוצים.
- 🎚️ **רב-רצועתי, רב-ציר זמן.** כמה עריכות בפרויקט אחד. מצבי מיזוג, חיתוך, היפוכים.
- 📝 **כותרות.** גופנים, קו מתאר, צל, לוח רקע. הגדרות מוכנות להתחיל מהן.
- 🎙️ **ניקוי קול במתג אחד.** הפחתת רעש, חיזוק הקול, איזון עוצמה. ועוד סנאי, רובוט, טלפון וחברים.
- 📤 **ייצוא.** H.264, HEVC, AV1. עד 4K 60, צבע 10 ביט.
- 🦀 **מנוע Rust נייטיב.** קומפוזיטור GPU, פרוקסי, פענוח חומרה. 4K נגלל חלק.
- 🤖 **ניתן לתסריט.** API של JSON-RPC, gRPC ו-MCP, ועוד CLI. סוכני AI יכולים לערוך איתו וידאו.
- 🖥️ **macOS, Windows, Linux, Android.** 14 שפות. אותה אפליקציה, אותם קובצי פרויקט.

## הורדה

שתי דרכים פנימה:

1. **[האתר](https://concatenate.pages.dev/#download)** נותן לכם את הגרסה הנכונה למחשב שלכם. התחילו כאן.
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** מכיל כל גרסה לכל פלטפורמה, עם מתקינים, חבילות וסכומי ביקורת. למקרה שתרצו לבחור בעצמכם.

Concat נמצא ב**בטא**: הוא עובד, ועדיין יש לו קצוות. [ספרו](https://github.com/jub0t/Concat/issues) כשתמצאו אחד.

**פלטפורמות**

- ✅ **Windows** · x86_64 ו-ARM. תוכנית התקנה ו-`.msi`. אם SmartScreen עוצר גרסה לא חתומה: **מידע נוסף** › **הפעל בכל זאת**
- ✅ **macOS** · Intel ו-Apple Silicon. אם macOS מסרב לפתוח גרסה לא חתומה: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 ו-ARM. ‏`.deb`, ‏`.rpm`, ‏`.AppImage` וחבילת Arch. וגם Flatpak ב-**[Flatpark](https://flatpark.org/apps/app.concat.editor/)**, מאגר Flatpak קהילתי שעוטף את ה-`.deb` ל-x86_64 של כל גרסה ומתעדכן איתה
- ✅ **Android** · טלפונים וטאבלטים
- ✅ **iOS / iPadOS** · iPhone ו-iPad, בהתקנה צדדית

✅ נתמך · 🚧 בעבודה · 🧪 טרם נבדק

**דרישות המערכת** וגודלי המודלים האופציונליים נמצאים [באתר](https://concatenate.pages.dev/guides/system-requirements).

## מתחילים

מורידים, פותחים, גוררים חומר, חותכים. בלי חשבון, בלי הגדרה.

**לדווח על משהו:** כל הרצה כותבת יומן, ובהגדרות › אודות יש כפתור שפותח אותו לצד הכפתור שמעתיק את פרטי המערכת. צרפו את שניהם ל-[issue](https://github.com/jub0t/Concat/issues) והדיווח מגיע עם כל מה שהוא צריך. עשר ההרצות האחרונות נשמרות, כך שזו של אתמול עדיין שם; שום דבר לא נשלח לשום מקום מעצמו.

## איך לתרום

> [!IMPORTANT]
> הדרך הטובה ביותר לתרום היא לקחת גרסה מעמוד ה-[Releases](https://github.com/jub0t/Concat/releases) ולהשתמש בה: למצוא איפה היא נשברת, ולומר איפה היא יכולה להיות טובה יותר.
>
> מוכנים לכתוב קוד? [CONTRIBUTING.md](../CONTRIBUTING.md) מכסה את ההתקנה, מבנה העץ, הבדיקות שצריך להריץ ואת רישוי התרומות. מפעילים את Concat מסקריפט, משירות או מסוכן? [תיעוד המפתחים](https://concatenate.pages.dev/docs) מכסה את ה-API של Concat ואת ערוצי התעבורה שלו: JSON-RPC, gRPC ו-MCP. [בדיון הזה](https://github.com/jub0t/Concat/discussions/3) הוכרז הפרויקט.
> 
> תורמים מוזמנים לבקש תפקיד `@Contributor` בשרת ה-Discord, רק צריך לבקש.

## תורמים

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## היסטוריית כוכבים

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&theme=dark&legend=bottom-right" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
 </picture>
</a>

<a id="sponsoring"></a>
## חסות

ל-Concat אין חומת תשלום ולעולם לא תהיה: בלי סימן מים, בלי חשבון, בלי רמה בתשלום. החסות היא מה שבא במקומה. אם Concat החליף אצלכם מנוי, חלק קטן ממנו מחזיק אותו בחיים.

**לאן זה הולך**

[מפת הדרכים](https://concatenate.pages.dev/roadmap) מפרטת מה החסות מממנת וכמה עולה כל חלק.

בחרו רמה [באתר](https://concatenate.pages.dev/#sponsor).

<a href="https://concatenate.pages.dev/#sponsor"><img src="https://img.shields.io/badge/Sponsor-Concat-0568FD?style=flat-square&labelColor=212123" alt="Sponsor Concat on the website" /></a>

לא יכולים לתרום כרגע? גם כוכב, דיווח על באג או מילה למישהו שעורך וידאו שווים הרבה.
