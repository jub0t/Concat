<p align="center">
  <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark-dark.png" alt="Concat" width="100" />
</p>

<h1 align="center">Concat</h1>
<h3 align="center">L’éditeur vidéo gratuit, open source et multiplateforme.</h3>

<div align="center">

<img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/editor.png" alt="Concat editor" width="100%" />

<p align="center">
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/github/downloads/jub0t/concat/total?style=flat&logo=github&logoColor=F8F8F8&label=Downloads&labelColor=212123&color=0568FD" alt="Total Downloads" /></a>
  <a href="https://github.com/jub0t/Concat/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/jub0t/Concat/ci.yml?style=flat&logo=githubactions&logoColor=F8F8F8&label=Build&labelColor=000000" alt="Build Status" /></a>
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/badge/Version-0.2.12-0568FD?style=flat&logo=semver&logoColor=F8F8F8&labelColor=212123" alt="Concat Version 0.2.12" /></a>
  <a href="https://discord.gg/DVuPfpXfqP"><img src="https://img.shields.io/badge/Discord-Join%20the%20server-5865F2?style=flat&logo=discord&logoColor=F8F8F8&labelColor=000000" alt="Join Concat Discord" /></a>
  <a href="../LICENSE"><img src="https://img.shields.io/badge/License-AGPL%20v3-0568FD?style=flat&logo=gnu&logoColor=F8F8F8&labelColor=212123" alt="License: AGPL-3.0-or-later" /></a>
</p>

<p align="center">
  <a href="https://www.star-history.com/jub0t/concat"><img src="https://api.star-history.com/badge?repo=jub0t/concat&type=rank" alt="Star History Rank" height="62" /></a>
  <a href="https://concat.standalones.org/#download"><img src="../assets/download_button.svg" alt="Download Concat" width="251" /></a>
  <a href="https://trendshift.io/repositories/191379?utm_source=repository-badge&amp;utm_medium=badge&amp;utm_campaign=badge-repository-191379" target="_blank" rel="noopener noreferrer"><img src="https://trendshift.io/api/badge/repositories/191379" alt="jub0t%2FConcat | Trendshift" width="250" height="55"/></a>
</p>

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · **🇫🇷 Français** · [🇮🇹 Italiano](README.it.md) · [🇳🇱 Nederlands](README.nl.md) · [🇵🇱 Polski](README.pl.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇦🇿 Azərbaycanca](README.az.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇭🇺 Magyar](README.hu.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇹🇭 ไทย](README.th.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md) · [🇧🇩 বাংলা](README.bn.md)

> Traduit du README anglais le 11 octobre 2026. La version anglaise est celle qui est tenue à jour.

## Sponsors payants

<table width="100%">
  <tr>
    <td align="center">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>Proxys à l’usage pour les développeurs<br />Code <code>JUB0T</code> pour 20 % de réduction</sub>
    </td>
  </tr>
</table>

</div>


## À propos

Concat est un éditeur vidéo gratuit et open source pour macOS, Windows, Linux, Android et iOS. Il couvre ce dont la plupart des montages ont besoin : sous-titres automatiques, isolation de la voix, synthèse vocale, suppression d’arrière-plan, animations et images clés, effets et titres, montage multipiste, export 4K. Sans aucun piège : pas de filigrane, pas de compte, pas d’abonnement, pas d’envoi.

Tout tourne en local sur un moteur natif en Rust avec un compositeur GPU. Installez, déposez vos rushes, montez. Les modèles d’IA pour les sous-titres, les voix et le détourage se téléchargent une fois depuis les Réglages et fonctionnent ensuite hors ligne. Vos rushes ne quittent jamais votre disque.

**Idéal pour :** TikTok, Reels et Shorts, vidéos YouTube, tutoriels et enregistrements d’écran, extraits de podcast, mèmes.

**Aussi pour les machines :** une API JSON-RPC, pour que les scripts et les agents IA puissent eux aussi monter de la vidéo avec.

## Points forts

- 🚫 **Pas de filigrane. Pas de compte. Pas de paywall.** Jamais.
- 🔒 **100 % local.** Rien n’est envoyé, l’IA comprise. Fonctionne hors ligne.
- 💬 **Sous-titres automatiques.** Dans la langue que vous parlez, générés sur votre machine, stylisés et déjà sur la timeline.
- 🎤 **Isolation de la voix.** Sortez une voix de sa musique ou de son bruit, ou retirez-la, chaque partie à son propre niveau.
- 🗣️ **Synthèse vocale + clonage de voix.** Une voix locale gratuite, ou n’importe quelle voix à partir de quelques secondes d’enregistrement. Sur ordinateur.
- 🧍 **Suppression d’arrière-plan, incrustation chroma et masques.** Personnes et objets détourés pour vous, un fond vert qui s’incruste tout seul, et une pile de masques sur chaque clip, chacun avec son propre mode et ses images clés.
- 🌀 **Animations d’entrée, de sortie et en boucle.** Pour les clips, les images, les titres et les formes. Chacune se joue sur sa carte avant que vous la choisissiez.
- ✨ **Plus de 100 effets, filtres et transitions, 25 effets sonores.** Looks cinéma, secousses et impacts, rétro, volets, le tout en direct dans l’aperçu. Une transition est un clip que vous déplacez et ajustez comme les autres.
- 🎞️ **Images clés.** Position, échelle, taille, rotation, opacité, volume, paramètres des effets. Éditeur de courbes intégré.
- 🏃 **Rampes et courbes de vitesse.** Constantes ou dessinées, douces ou droites. Arrêts sur image et lecture inversée.
- 🪄 **Améliorer et Stabiliser.** Restaurez des images floues ou anciennes, jusqu’en 2K. Calmez un plan tremblant.
- ✂️ **Coupez vite, coupez juste.** Diviser, ajuster, ripple, fusionner, sous-titres et incrustations liés, une timeline magnétique si vous la voulez. Chaque coupe tombe sur l’image, à l’identique dans l’aperçu et dans l’export.
- 🎚️ **Multipiste, multi-timeline.** Empilez vidéo, images fixes, titres et formes sur autant de pistes que vous voulez. Plusieurs montages dans un projet. Modes de fusion, recadrage, retournements.
- 📝 **Titres et formes.** Polices, contour, ombre, cartouches, emoji en couleur et préréglages pour l’arabe, le chinois, l’hindi, le thaï et huit autres langues. Formes avec dégradés, étoiles, contours et ombres.
- 🎙️ **Voix off et nettoyage de la voix en un clic.** Enregistrez par-dessus votre montage. Réduction du bruit, amélioration de la voix, égalisation du volume. Plus écureuil, robot, téléphone et compagnie.
- 📤 **Export.** H.264, HEVC, AV1, avec l’encodeur propre à votre ordinateur. Jusqu’à la 4K 60, couleur 10 bits.
- ⚡ **Rapide.** Proxys intégrés, pour que la 4K défile sans à-coups. Les titres se dessinent sur tous les cœurs, et chaque effet est prêt dès l’ouverture de la fenêtre.
- 🛟 **Continue sur n’importe quelle machine.** Quand la puce graphique lâche, l’aperçu continue de s’afficher.
- 🤖 **Scriptable.** Une API JSON-RPC avec laquelle les scripts et les agents IA peuvent monter de la vidéo. Une compilation depuis les sources ajoute gRPC et un outil en ligne de commande.
- 🖥️ **macOS, Windows, Linux, Android, iOS.** 27 langues. Même app, mêmes fichiers de projet.

## Téléchargement

Deux portes d’entrée :

1. **[Le site web](https://concat.standalones.org/#download)** vous donne la bonne version pour votre machine. Commencez ici.
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** propose chaque version pour chaque plateforme, avec installeurs, paquets et sommes de contrôle. Pour quand vous voulez choisir.

Concat est en **bêta** : ça marche, et il reste des angles. [Dites-le](https://github.com/jub0t/Concat/issues) quand vous en trouvez un.

**Plateformes**

- ✅ **Windows** · x86_64 (les versions ARM sont en pause, voir [#326](https://github.com/jub0t/Concat/issues/326)). Un installeur et un `.msi`. Si SmartScreen bloque une version non signée : **Informations complémentaires** › **Exécuter quand même**
- ✅ **macOS** · Intel et Apple Silicon. Si macOS refuse d’ouvrir une version non signée : `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 et ARM. `.deb`, `.rpm`, `.AppImage` et un paquet Arch. Aussi un Flatpak sur **[Flatpark](https://flatpark.org/apps/app.concat.editor/)**, un dépôt Flatpak communautaire qui empaquette le `.deb` x86_64 de chaque version et se met à jour avec elle
- ✅ **Android** · téléphones et tablettes
- ✅ **iOS / iPadOS** · iPhone et iPad, en sideload

✅ Pris en charge · 🚧 En cours · 🧪 À tester

La **configuration requise** et la taille des modèles optionnels sont sur [le site web](https://concat.standalones.org/guides/system-requirements).

**[Code signing policy](../CODE_SIGNING.md)**

## Contribuer

> [!IMPORTANT]
> La meilleure façon de contribuer est de prendre une version sur la page [Releases](https://github.com/jub0t/Concat/releases) et de s’en servir : trouvez où ça casse, et dites où ça pourrait être mieux.
>
> Prêt à écrire du code ? [CONTRIBUTING.md](../CONTRIBUTING.md) couvre l’installation, l’organisation de l’arborescence, les vérifications à lancer et la licence des contributions. Vous pilotez Concat depuis un script, un service ou un agent ? [La documentation développeur](https://concat.standalones.org/docs) couvre l’API Concat et ses transports : JSON-RPC et gRPC. [Cette Discussion](https://github.com/jub0t/Concat/discussions/3) est celle où le projet a été annoncé.
> 
> Les contributeurs peuvent obtenir un rôle `@Contributor` sur le serveur Discord : il suffit de le demander.

## Historique des étoiles

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
</a>

## Contributeurs

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## DMCA

Si vous pensez que quelque chose dans ce dépôt, dans ses versions publiées, ou dans un paquet ou un modèle qu’il distribue porte atteinte à vos droits d’auteur, écrivez à [jub0trd@gmail.com](mailto:jub0trd@gmail.com) en indiquant une description de l’œuvre dont vous êtes titulaire, un lien vers l’endroit où le contenu litigieux apparaît ici, vos coordonnées, une déclaration selon laquelle vous estimez de bonne foi que cet usage n’est autorisé ni par vous, ni par votre mandataire, ni par la loi, et une déclaration, sous peine de parjure, attestant que la notification est exacte et que vous êtes le titulaire des droits ou habilité à agir en son nom, revêtue de votre signature manuscrite ou électronique. Les notifications valides sont traitées sans délai, et toute personne dont le contenu a été retiré peut envoyer une contre-notification à la même adresse.
