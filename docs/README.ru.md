<div align="center">
<table width="100%">
  <tr>
    <td align="left" width="120">
      <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark.png" alt="Concat" width="100" />
    </td>
    <td align="">
      <h1>Concat</h1>
      <h3 style="margin-top: -10px;">По-настоящему бесплатная кроссплатформенная замена CapCut с открытым исходным кодом.</h3>
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

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · **🇷🇺 Русский** · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md)

> Переведено с английского README 2 октября 2026 года. Актуальной поддерживается английская версия.

## Платные спонсоры

<table width="100%">
  <tr>
    <td align="center" width="50%">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>Прокси с оплатой по факту для разработчиков<br />Код <code>JUB0T</code> даёт скидку 20 %</sub>
    </td>
    <td align="center" width="50%">
      <a href="#sponsoring"><b>Ваш логотип здесь</b></a><br />
      <sub><a href="#sponsoring">Поддержите Concat</a> и ваше имя, логотип и ссылка займут это место</sub>
    </td>
  </tr>
</table>

</div>


## О проекте

Concat - бесплатный видеоредактор с открытым исходным кодом, альтернатива CapCut для macOS, Windows, Linux и Android. Он умеет то, ради чего люди на самом деле открывают CapCut: автоматические субтитры, синтез речи, удаление фона, анимацию по ключевым кадрам, эффекты и титры, многодорожечный монтаж, экспорт в 4K. И без подвохов: ни водяного знака, ни аккаунта, ни подписки, ни загрузки на сервер.

Всё работает локально на нативном движке на Rust с GPU-композитором. Установите, перетащите материал, режьте. Модели ИИ для субтитров, голосов и вырезания фона скачиваются один раз из настроек и дальше работают офлайн. Ваш материал никогда не покидает ваш диск.

**Подходит для:** TikTok, Reels и Shorts, видео на YouTube, обучающих роликов и записей экрана, фрагментов подкастов, мемов.

**И для машин тоже:** API JSON-RPC, gRPC и MCP, чтобы скрипты и ИИ-агенты тоже могли монтировать в нём видео.

## Главное

- 🚫 **Без водяных знаков. Без аккаунта. Без платного доступа.** Никогда.
- 🔒 **100 % локально.** Ничего не загружается. Работает офлайн.
- 💬 **Автоматические субтитры.** Локальный Whisper. Выберите размер модели - и стилизованные субтитры окажутся на таймлайне.
- 🗣️ **Синтез речи + клонирование голоса.** Бесплатные локальные голоса или любой голос по нескольким секундам записи.
- 🧍 **Удаление фона.** Люди, объекты или нарисуйте маску сами.
- 🎞️ **Ключевые кадры.** Положение, масштаб, поворот, прозрачность, громкость, параметры эффектов. Встроенный редактор кривых.
- ✨ **Более 170 эффектов, фильтров, переходов и анимаций текста.** Рендер на GPU, вживую в предпросмотре.
- ✂️ **Быстрый монтаж.** Разрезать, подрезать, ripple, объединить, стоп-кадр, скорость. Магнитный таймлайн по желанию.
- 🎚️ **Много дорожек, много таймлайнов.** Несколько монтажей в одном проекте. Режимы наложения, кадрирование, отражения.
- 📝 **Титры.** Шрифты, обводка, тень, подложка. Пресеты для старта.
- 🎙️ **Чистка голоса одним переключателем.** Шумоподавление, улучшение голоса, выравнивание громкости. Плюс бурундук, робот, телефон и компания.
- 📤 **Экспорт.** H.264, HEVC, AV1. До 4K 60, 10-битный цвет.
- 🦀 **Нативный движок на Rust.** GPU-композитор, прокси, аппаратное декодирование. 4K перематывается плавно.
- 🤖 **Скриптуемый.** API JSON-RPC, gRPC и MCP плюс CLI. ИИ-агенты могут монтировать в нём видео.
- 🖥️ **macOS, Windows, Linux, Android.** 14 языков. Одно приложение, одни и те же файлы проектов.

## Скачать

Два пути:

1. **[Сайт](https://concatenate.pages.dev/#download)** подберёт сборку под вашу машину. Начните отсюда.
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** содержит каждую сборку для каждой платформы, с установщиками, пакетами и контрольными суммами. Когда хочется выбрать самому.

Concat в **бете**: он работает, но шероховатости ещё есть. [Сообщите](https://github.com/jub0t/Concat/issues), когда найдёте одну.

**Платформы**

- ✅ **Windows** · x86_64 и ARM. Установщик и `.msi`. Если SmartScreen останавливает неподписанную сборку: **Подробнее** › **Выполнить в любом случае**
- ✅ **macOS** · Intel и Apple Silicon. Если macOS отказывается открыть неподписанную сборку: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 и ARM. `.deb`, `.rpm`, `.AppImage` и пакет для Arch. А ещё Flatpak на **[Flatpark](https://flatpark.org/apps/app.concat.editor/)** - сообществом поддерживаемом Flatpak-репозитории, который оборачивает `.deb` x86_64 каждого релиза и обновляется вместе с ним
- ✅ **Android** · телефоны и планшеты
- ✅ **iOS / iPadOS** · iPhone и iPad, через sideload

✅ Поддерживается · 🚧 В работе · 🧪 Предстоит проверить

**Системные требования** и размеры необязательных моделей - на [сайте](https://concatenate.pages.dev/guides/system-requirements).

## Начало работы

Скачайте, откройте, перетащите материал, режьте. Без аккаунта, без настройки.

**Чтобы сообщить о проблеме:** каждый запуск пишет журнал, а в Настройки › О программе есть кнопка, которая его открывает, рядом с той, что копирует сведения о системе. Приложите и то и другое к [issue](https://github.com/jub0t/Concat/issues) - и отчёт придёт со всем необходимым. Хранятся последние десять запусков, так что вчерашний ещё на месте; ничего никуда не отправляется само.

## Как помочь

> [!IMPORTANT]
> Лучший способ помочь - взять сборку со страницы [Releases](https://github.com/jub0t/Concat/releases) и пользоваться ею: найти, где она ломается, и сказать, где могла бы быть лучше.
>
> Готовы писать код? В [CONTRIBUTING.md](../CONTRIBUTING.md) описаны настройка, устройство дерева, какие проверки запускать и как лицензируются вклады. Управляете Concat из скрипта, сервиса или агента? [Документация для разработчиков](https://concatenate.pages.dev/docs) описывает API Concat и его транспорты: JSON-RPC, gRPC и MCP. [В этом обсуждении](https://github.com/jub0t/Concat/discussions/3) проект был анонсирован.
> 
> Участники могут получить роль `@Contributor` на сервере Discord - достаточно попросить.

## Участники

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## История звёзд

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&theme=dark&legend=bottom-right" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
 </picture>
</a>

<a id="sponsoring"></a>
## Спонсорство

У Concat нет платного доступа и никогда не будет: ни водяного знака, ни аккаунта, ни платного тарифа. Его место занимает спонсорство. Если Concat заменил вам подписку, малая её часть поддерживает его на плаву.

**Куда идут деньги**

[Дорожная карта](https://concatenate.pages.dev/roadmap) показывает, что оплачивает спонсорство и сколько стоит каждая часть.

Выберите уровень на [сайте](https://concatenate.pages.dev/#sponsor).

<a href="https://concatenate.pages.dev/#sponsor"><img src="https://img.shields.io/badge/Sponsor-Concat-0568FD?style=flat-square&labelColor=212123" alt="Sponsor Concat on the website" /></a>

Нет возможности помочь деньгами? Звезда, сообщение об ошибке или слово тому, кто монтирует видео, тоже значат очень много.
