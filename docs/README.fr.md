<div align="center">
<table width="100%">
  <tr>
    <td align="left" width="120">
      <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark.png" alt="Concat" width="100" />
    </td>
    <td align="">
      <h1>Concat</h1>
      <h3 style="margin-top: -10px;">Le remplaçant de CapCut vraiment gratuit, open source et multiplateforme.</h3>
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

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · **🇫🇷 Français** · [🇮🇹 Italiano](README.it.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md)

> Traduit du README anglais le 2 octobre 2026. La version anglaise est celle qui est tenue à jour.

## Sponsors payants

<table width="100%">
  <tr>
    <td align="center" width="50%">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>Proxys à l’usage pour les développeurs<br />Code <code>JUB0T</code> pour 20 % de réduction</sub>
    </td>
    <td align="center" width="50%">
      <a href="#sponsoring"><b>Votre logo ici</b></a><br />
      <sub><a href="#sponsoring">Sponsorisez Concat</a> et votre nom, votre logo et votre lien prennent cette place</sub>
    </td>
  </tr>
</table>

</div>


## À propos

Concat est un éditeur vidéo gratuit et open source, une alternative à CapCut pour macOS, Windows, Linux et Android. Il couvre ce pour quoi on ouvre vraiment CapCut : sous-titres automatiques, synthèse vocale, suppression d’arrière-plan, animation par images clés, effets et titres, montage multipiste, export 4K. Sans aucun piège : pas de filigrane, pas de compte, pas d’abonnement, pas d’envoi.

Tout tourne en local sur un moteur natif en Rust avec un compositeur GPU. Installez, déposez vos rushes, montez. Les modèles d’IA pour les sous-titres, les voix et le détourage se téléchargent une fois depuis les Réglages et fonctionnent ensuite hors ligne. Vos rushes ne quittent jamais votre disque.

**Idéal pour :** TikTok, Reels et Shorts, vidéos YouTube, tutoriels et captures d’écran, extraits de podcast, mèmes.

**Aussi pour les machines :** une API JSON-RPC, gRPC et MCP, pour que les scripts et les agents IA puissent eux aussi monter de la vidéo avec.

## Points forts

- 🚫 **Pas de filigrane. Pas de compte. Pas de paywall.** Jamais.
- 🔒 **100 % local.** Rien n’est envoyé. Fonctionne hors ligne.
- 💬 **Sous-titres automatiques.** Whisper en local. Choisissez une taille de modèle, obtenez des sous-titres stylés sur la timeline.
- 🗣️ **Synthèse vocale + clonage de voix.** Voix locales gratuites, ou n’importe quelle voix à partir de quelques secondes d’enregistrement.
- 🧍 **Suppression d’arrière-plan.** Personnes, objets, ou peignez le masque vous-même.
- 🎞️ **Images clés.** Position, échelle, rotation, opacité, volume, paramètres d’effets. Éditeur de courbes intégré.
- ✨ **Plus de 170 effets, filtres, transitions et animations de texte.** Rendus par le GPU, en direct dans l’aperçu.
- ✂️ **Montez vite.** Couper, rogner, ripple, fusionner, arrêt sur image, vitesse. Timeline magnétique si vous la voulez.
- 🎚️ **Multipiste, multi-timeline.** Plusieurs montages dans un projet. Modes de fusion, recadrage, retournements.
- 📝 **Titres.** Polices, contour, ombre, plaque de fond. Des préréglages pour démarrer.
- 🎙️ **Nettoyage de voix d’un seul geste.** Débruiter, rehausser la voix, niveler le volume. Plus écureuil, robot, téléphone et compagnie.
- 📤 **Export.** H.264, HEVC, AV1. Jusqu’à 4K 60, couleur 10 bits.
- 🦀 **Moteur natif en Rust.** Compositeur GPU, proxys, décodage matériel. La 4K se parcourt sans accroc.
- 🤖 **Scriptable.** API JSON-RPC, gRPC et MCP, plus une CLI. Les agents IA peuvent monter de la vidéo avec.
- 🖥️ **macOS, Windows, Linux, Android.** 14 langues. La même appli, les mêmes fichiers de projet.

## Téléchargement

Deux portes d’entrée :

1. **[Le site web](https://concatenate.pages.dev/#download)** vous donne la bonne version pour votre machine. Commencez ici.
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** propose chaque version pour chaque plateforme, avec installeurs, paquets et sommes de contrôle. Pour quand vous voulez choisir.

Concat est en **bêta** : ça marche, et il reste des angles. [Dites-le](https://github.com/jub0t/Concat/issues) quand vous en trouvez un.

**Plateformes**

- ✅ **Windows** · x86_64 et ARM. Un installeur et un `.msi`. Si SmartScreen bloque une version non signée : **Informations complémentaires** › **Exécuter quand même**
- ✅ **macOS** · Intel et Apple Silicon. Si macOS refuse d’ouvrir une version non signée : `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 et ARM. `.deb`, `.rpm`, `.AppImage` et un paquet Arch. Aussi un Flatpak sur **[Flatpark](https://flatpark.org/apps/app.concat.editor/)**, un dépôt Flatpak communautaire qui empaquette le `.deb` x86_64 de chaque version et se met à jour avec elle
- ✅ **Android** · téléphones et tablettes
- ✅ **iOS / iPadOS** · iPhone et iPad, en sideload

✅ Pris en charge · 🚧 En cours · 🧪 À tester

La **configuration requise** et la taille des modèles optionnels sont sur [le site web](https://concatenate.pages.dev/guides/system-requirements).

## Premiers pas

Téléchargez, ouvrez, déposez vos rushes, montez. Pas de compte, pas de configuration.

**Pour signaler quelque chose :** chaque lancement écrit un journal, et Réglages › À propos a le bouton qui l’ouvre, à côté de celui qui copie les informations de votre système. Joignez les deux à une [issue](https://github.com/jub0t/Concat/issues) et le rapport arrive avec tout ce qu’il lui faut. Les dix derniers lancements sont conservés, celui d’hier est donc encore là ; rien n’est jamais envoyé nulle part tout seul.

## Contribuer

> [!IMPORTANT]
> La meilleure façon de contribuer est de prendre une version sur la page [Releases](https://github.com/jub0t/Concat/releases) et de s’en servir : trouvez où ça casse, et dites où ça pourrait être mieux.
>
> Prêt à écrire du code ? [CONTRIBUTING.md](../CONTRIBUTING.md) couvre l’installation, l’organisation de l’arborescence, les vérifications à lancer et la licence des contributions. Vous pilotez Concat depuis un script, un service ou un agent ? [La documentation développeur](https://concatenate.pages.dev/docs) couvre l’API Concat et ses transports : JSON-RPC, gRPC et MCP. [Cette Discussion](https://github.com/jub0t/Concat/discussions/3) est celle où le projet a été annoncé.
> 
> Les contributeurs peuvent demander un rôle `@Contributor` sur le serveur Discord, il suffit de le demander.

## Contributeurs

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## Historique des étoiles

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&theme=dark&legend=bottom-right" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
 </picture>
</a>

<a id="sponsoring"></a>
## Sponsoring

Concat n’a pas de paywall et n’en aura jamais : pas de filigrane, pas de compte, pas d’offre payante. Le sponsoring en tient lieu. Si Concat a remplacé un abonnement pour vous, une fraction de celui-ci le fait vivre.

**Où ça va**

La [feuille de route](https://concatenate.pages.dev/roadmap) détaille ce que le sponsoring finance, et ce que coûte chaque pièce.

Choisissez un palier sur [le site web](https://concatenate.pages.dev/#sponsor).

<a href="https://concatenate.pages.dev/#sponsor"><img src="https://img.shields.io/badge/Sponsor-Concat-0568FD?style=flat-square&labelColor=212123" alt="Sponsor Concat on the website" /></a>

Pas en mesure de participer ? Une étoile, un rapport de bug ou un mot à quelqu’un qui monte de la vidéo comptent beaucoup aussi.
