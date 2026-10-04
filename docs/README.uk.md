<div align="center">
<table width="100%">
  <tr>
    <td align="left" width="120">
      <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark.png" alt="Concat" width="100" />
    </td>
    <td align="">
      <h1>Concat</h1>
      <h3 style="margin-top: -10px;">По-справжньому безкоштовна кросплатформна заміна CapCut з відкритим кодом.</h3>
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

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · **🇺🇦 Українська** · [🇹🇷 Türkçe](README.tr.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md)

> Перекладено з англійського README 2 жовтня 2026 року. Актуальною підтримується англійська версія.

## Платні спонсори

<table width="100%">
  <tr>
    <td align="center" width="50%">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>Проксі з оплатою за використання для розробників<br />Код <code>JUB0T</code> дає знижку 20 %</sub>
    </td>
    <td align="center" width="50%">
      <a href="#sponsoring"><b>Ваш логотип тут</b></a><br />
      <sub><a href="#sponsoring">Підтримайте Concat</a> і ваше ім’я, логотип і посилання займуть це місце</sub>
    </td>
  </tr>
</table>

</div>


## Про проєкт

Concat - безкоштовний відеоредактор з відкритим кодом, альтернатива CapCut для macOS, Windows, Linux та Android. Він уміє те, заради чого люди насправді відкривають CapCut: автоматичні субтитри, синтез мовлення, видалення фону, анімацію за ключовими кадрами, ефекти й титри, багатодоріжковий монтаж, експорт у 4K. І без пасток: ані водяного знака, ані облікового запису, ані підписки, ані завантаження на сервер.

Усе працює локально на нативному рушії на Rust із GPU-композитором. Встановіть, перетягніть матеріал, ріжте. Моделі ШІ для субтитрів, голосів і вирізання фону завантажуються один раз із налаштувань і далі працюють офлайн. Ваш матеріал ніколи не залишає ваш диск.

**Підходить для:** TikTok, Reels і Shorts, відео на YouTube, навчальних роликів і записів екрана, фрагментів подкастів, мемів.

**І для машин теж:** API JSON-RPC, gRPC і MCP, щоб скрипти та ШІ-агенти теж могли монтувати в ньому відео.

## Головне

- 🚫 **Без водяних знаків. Без облікового запису. Без платного доступу.** Ніколи.
- 🔒 **100 % локально.** Нічого не завантажується. Працює офлайн.
- 💬 **Автоматичні субтитри.** Локальний Whisper. Оберіть розмір моделі - і стилізовані субтитри опиняться на таймлайні.
- 🗣️ **Синтез мовлення + клонування голосу.** Безкоштовні локальні голоси або будь-який голос із кількох секунд запису.
- 🧍 **Видалення фону.** Люди, об’єкти або намалюйте маску самі.
- 🎞️ **Ключові кадри.** Положення, масштаб, поворот, прозорість, гучність, параметри ефектів. Вбудований редактор кривих.
- ✨ **Понад 170 ефектів, фільтрів, переходів і анімацій тексту.** Рендер на GPU, наживо в попередньому перегляді.
- ✂️ **Швидкий монтаж.** Розрізати, підрізати, ripple, об’єднати, стоп-кадр, швидкість. Магнітний таймлайн за бажанням.
- 🎚️ **Багато доріжок, багато таймлайнів.** Кілька монтажів в одному проєкті. Режими накладання, кадрування, віддзеркалення.
- 📝 **Титри.** Шрифти, обведення, тінь, підкладка. Пресети для старту.
- 🎙️ **Чищення голосу одним перемикачем.** Шумозаглушення, покращення голосу, вирівнювання гучності. Плюс бурундук, робот, телефон і компанія.
- 📤 **Експорт.** H.264, HEVC, AV1. До 4K 60, 10-бітний колір.
- 🦀 **Нативний рушій на Rust.** GPU-композитор, проксі, апаратне декодування. 4K перемотується плавно.
- 🤖 **Скриптований.** API JSON-RPC, gRPC і MCP плюс CLI. ШІ-агенти можуть монтувати в ньому відео.
- 🖥️ **macOS, Windows, Linux, Android.** 14 мов. Один застосунок, ті самі файли проєктів.

## Завантажити

Два шляхи:

1. **[Сайт](https://concatenate.pages.dev/#download)** добере збірку під вашу машину. Почніть звідси.
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** містить кожну збірку для кожної платформи, з інсталяторами, пакетами й контрольними сумами. Коли хочеться обрати самому.

Concat у **беті**: він працює, але шорсткості ще є. [Повідомте](https://github.com/jub0t/Concat/issues), коли знайдете одну.

**Платформи**

- ✅ **Windows** · x86_64 і ARM. Інсталятор і `.msi`. Якщо SmartScreen зупиняє непідписану збірку: **Докладніше** › **Все одно запустити**
- ✅ **macOS** · Intel і Apple Silicon. Якщо macOS відмовляється відкрити непідписану збірку: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 і ARM. `.deb`, `.rpm`, `.AppImage` і пакет для Arch. А ще Flatpak на **[Flatpark](https://flatpark.org/apps/app.concat.editor/)** - спільнотному Flatpak-репозиторії, який обгортає `.deb` x86_64 кожного релізу й оновлюється разом із ним
- ✅ **Android** · телефони та планшети
- ✅ **iOS / iPadOS** · iPhone та iPad, через sideload

✅ Підтримується · 🚧 У роботі · 🧪 Треба перевірити

**Системні вимоги** та розміри необов’язкових моделей - на [сайті](https://concatenate.pages.dev/guides/system-requirements).

## Початок роботи

Завантажте, відкрийте, перетягніть матеріал, ріжте. Без облікового запису, без налаштування.

**Щоб повідомити про проблему:** кожен запуск пише журнал, а в Налаштування › Про програму є кнопка, яка його відкриває, поруч із тією, що копіює відомості про систему. Додайте обидва до [issue](https://github.com/jub0t/Concat/issues) - і звіт прийде з усім потрібним. Зберігаються останні десять запусків, тож учорашній ще на місці; нічого нікуди не надсилається саме по собі.

## Як долучитися

> [!IMPORTANT]
> Найкращий спосіб долучитися - взяти збірку зі сторінки [Releases](https://github.com/jub0t/Concat/releases) і користуватися нею: знайти, де вона ламається, і сказати, де могла б бути кращою.
>
> Готові писати код? У [CONTRIBUTING.md](../CONTRIBUTING.md) описано налаштування, будову дерева, які перевірки запускати та як ліцензуються внески. Керуєте Concat зі скрипту, сервісу чи агента? [Документація для розробників](https://concatenate.pages.dev/docs) описує API Concat та його транспорти: JSON-RPC, gRPC і MCP. [У цьому обговоренні](https://github.com/jub0t/Concat/discussions/3) проєкт було анонсовано.
> 
> Учасники можуть отримати роль `@Contributor` на сервері Discord - достатньо попросити.

## Учасники

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## Історія зірок

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&theme=dark&legend=bottom-right" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
 </picture>
</a>

<a id="sponsoring"></a>
## Спонсорство

У Concat немає платного доступу й ніколи не буде: ані водяного знака, ані облікового запису, ані платного тарифу. Його місце посідає спонсорство. Якщо Concat замінив вам підписку, мала її частина тримає його на плаву.

**Куди йдуть кошти**

[Дорожня карта](https://concatenate.pages.dev/roadmap) показує, що оплачує спонсорство і скільки коштує кожна частина.

Оберіть рівень на [сайті](https://concatenate.pages.dev/#sponsor).

<a href="https://concatenate.pages.dev/#sponsor"><img src="https://img.shields.io/badge/Sponsor-Concat-0568FD?style=flat-square&labelColor=212123" alt="Sponsor Concat on the website" /></a>

Немає змоги допомогти грошима? Зірка, повідомлення про помилку чи слово тому, хто монтує відео, теж важать дуже багато.
