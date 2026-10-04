<div align="center">
<table width="100%">
  <tr>
    <td align="left" width="120">
      <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark.png" alt="Concat" width="100" />
    </td>
    <td align="">
      <h1>Concat</h1>
      <h3 style="margin-top: -10px;">El reemplazo de CapCut realmente gratuito, de código abierto y multiplataforma.</h3>
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

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · **🇪🇸 Español** · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md)

> Traducido del README en inglés el 2 de octubre de 2026. La versión en inglés es la que se mantiene al día.

## Patrocinadores de pago

<table width="100%">
  <tr>
    <td align="center" width="50%">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>Proxies de pago por uso para desarrolladores<br />Usa el código <code>JUB0T</code> para un 20 % de descuento</sub>
    </td>
    <td align="center" width="50%">
      <a href="#sponsoring"><b>Tu logo aquí</b></a><br />
      <sub><a href="#sponsoring">Patrocina Concat</a> y tu nombre, tu logo y tu enlace ocupan este espacio</sub>
    </td>
  </tr>
</table>

</div>


## Acerca de

Concat es un editor de vídeo gratuito y de código abierto, una alternativa a CapCut para macOS, Windows, Linux y Android. Cubre lo que la gente realmente abre CapCut para hacer: subtítulos automáticos, texto a voz, eliminación de fondo, animación por fotogramas clave, efectos y títulos, corte multipista, exportación 4K. Sin trampas: sin marca de agua, sin cuenta, sin suscripción, sin subir nada.

Todo corre en local sobre un motor nativo en Rust con un compositor por GPU. Instálalo, suelta el material, corta. Los modelos de IA para subtítulos, voces y recorte se descargan una vez desde Ajustes y después funcionan sin conexión. Tu material nunca sale de tu disco.

**Bueno para:** TikTok, Reels y Shorts, vídeos de YouTube, tutoriales y grabaciones de pantalla, clips de pódcast, memes.

**También para máquinas:** una API JSON-RPC, gRPC y MCP, para que los scripts y los agentes de IA también puedan cortar vídeo con él.

## Lo más destacado

- 🚫 **Sin marcas de agua. Sin cuenta. Sin muro de pago.** Nunca.
- 🔒 **100 % local.** Nada se sube. Funciona sin conexión.
- 💬 **Subtítulos automáticos.** Whisper en local. Elige un tamaño de modelo y obtén subtítulos con estilo en la línea de tiempo.
- 🗣️ **Texto a voz + clonación de voz.** Voces locales gratuitas, o cualquier voz a partir de unos segundos de grabación.
- 🧍 **Eliminación de fondo.** Personas, objetos, o pinta la máscara tú mismo.
- 🎞️ **Fotogramas clave.** Posición, escala, rotación, opacidad, volumen, parámetros de efectos. Editor de curvas integrado.
- ✨ **Más de 170 efectos, filtros, transiciones y animaciones de texto.** Renderizados por GPU, en vivo en la vista previa.
- ✂️ **Corta rápido.** Dividir, recortar, ripple, unir, congelar fotograma, velocidad. Línea de tiempo magnética si la quieres.
- 🎚️ **Multipista, multilínea de tiempo.** Varios montajes en un proyecto. Modos de fusión, recorte, volteos.
- 📝 **Títulos.** Fuentes, trazo, sombra, placa de fondo. Preajustes para empezar.
- 🎙️ **Limpieza de voz con un interruptor.** Reducir ruido, realzar la voz, nivelar el volumen. Más ardilla, robot, teléfono y compañía.
- 📤 **Exportación.** H.264, HEVC, AV1. Hasta 4K 60, color de 10 bits.
- 🦀 **Motor nativo en Rust.** Compositor por GPU, proxies, decodificación por hardware. El 4K se desplaza con fluidez.
- 🤖 **Programable.** API JSON-RPC, gRPC y MCP, más una CLI. Los agentes de IA pueden cortar vídeo con él.
- 🖥️ **macOS, Windows, Linux, Android.** 14 idiomas. La misma app, los mismos archivos de proyecto.

## Descarga

Dos formas de entrar:

1. **[El sitio web](https://concatenate.pages.dev/#download)** te da la compilación adecuada para tu máquina. Empieza aquí.
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** tiene todas las compilaciones para todas las plataformas, con instaladores, paquetes y sumas de verificación. Para cuando quieras elegir.

Concat está en **beta**: funciona, y aún tiene aristas. [Cuéntalo](https://github.com/jub0t/Concat/issues) cuando encuentres una.

**Plataformas**

- ✅ **Windows** · x86_64 y ARM. Un instalador y un `.msi`. Si SmartScreen detiene una compilación sin firmar: **Más información** › **Ejecutar de todas formas**
- ✅ **macOS** · Intel y Apple Silicon. Si macOS se niega a abrir una compilación sin firmar: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 y ARM. `.deb`, `.rpm`, `.AppImage` y un paquete para Arch. También un Flatpak en **[Flatpark](https://flatpark.org/apps/app.concat.editor/)**, un remoto Flatpak comunitario que envuelve el `.deb` x86_64 de cada versión y se actualiza con ella
- ✅ **Android** · teléfonos y tabletas
- ✅ **iOS / iPadOS** · iPhone y iPad, por sideload

✅ Compatible · 🚧 En desarrollo · 🧪 Por probar

Los **requisitos del sistema** y los tamaños de los modelos opcionales están en [el sitio web](https://concatenate.pages.dev/guides/system-requirements).

## Primeros pasos

Descárgalo, ábrelo, suelta el material, corta. Sin cuenta, sin configuración.

**Para informar de algo:** cada ejecución escribe un registro, y en Ajustes › Acerca de está el botón que lo abre junto al que copia la información de tu sistema. Adjunta ambos a un [issue](https://github.com/jub0t/Concat/issues) y el informe llega con todo lo que necesita. Se conservan las últimas diez ejecuciones, así que la de ayer sigue ahí; nada se envía nunca a ningún sitio por sí solo.

## Cómo contribuir

> [!IMPORTANT]
> La mejor forma de contribuir es coger una compilación de la página de [Releases](https://github.com/jub0t/Concat/releases) y usarla: encuentra dónde falla y di dónde podría ser mejor.
>
> ¿Listo para escribir código? [CONTRIBUTING.md](../CONTRIBUTING.md) cubre la configuración, la disposición del árbol, las comprobaciones que ejecutar y cómo se licencian las contribuciones. ¿Controlas Concat desde un script, un servicio o un agente? [La documentación para desarrolladores](https://concatenate.pages.dev/docs) cubre la API de Concat y sus transportes: JSON-RPC, gRPC y MCP. [Esta Discussion](https://github.com/jub0t/Concat/discussions/3) es donde se anunció el proyecto.
> 
> Los colaboradores pueden reclamar un rol `@Contributor` en el servidor de Discord, solo hay que pedirlo.

## Colaboradores

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## Historial de estrellas

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&theme=dark&legend=bottom-right" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
 </picture>
</a>

<a id="sponsoring"></a>
## Patrocinio

Concat no tiene muro de pago y nunca lo tendrá: sin marca de agua, sin cuenta, sin nivel de pago. El patrocinio es lo que ocupa su lugar. Si Concat ha sustituido una suscripción para ti, una fracción de ella lo mantiene en marcha.

**A dónde va**

La [hoja de ruta](https://concatenate.pages.dev/roadmap) expone lo que paga el patrocinio y lo que cuesta cada pieza.

Elige un nivel en [el sitio web](https://concatenate.pages.dev/#sponsor).

<a href="https://concatenate.pages.dev/#sponsor"><img src="https://img.shields.io/badge/Sponsor-Concat-0568FD?style=flat-square&labelColor=212123" alt="Sponsor Concat on the website" /></a>

¿No puedes aportar ahora? Una estrella, un informe de error o una mención a alguien que edite vídeo también cuentan mucho.
