<p align="center">
  <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark-dark.png" alt="Concat" width="100" />
</p>

<h1 align="center">Concat</h1>
<h3 align="center">Il sostituto di CapCut gratuito, open source e multipiattaforma.</h3>

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

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · **🇮🇹 Italiano** · [🇳🇱 Nederlands](README.nl.md) · [🇵🇱 Polski](README.pl.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇦🇿 Azərbaycanca](README.az.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇹🇭 ไทย](README.th.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md) · [🇧🇩 বাংলা](README.bn.md)

> Tradotto dal README inglese l’11 ottobre 2026. La versione inglese è quella tenuta aggiornata.

## Sponsor paganti

<table width="100%">
  <tr>
    <td align="center">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>Proxy a consumo per sviluppatori<br />Usa il codice <code>JUB0T</code> per il 20 % di sconto</sub>
    </td>
  </tr>
</table>

</div>


## Informazioni

Concat è un editor video gratuito e open source, un’alternativa a CapCut per macOS, Windows, Linux e Android. Copre ciò per cui la gente apre davvero CapCut: sottotitoli automatici, sintesi vocale, rimozione dello sfondo, animazione con fotogrammi chiave, effetti e titoli, montaggio multitraccia, esportazione 4K. Senza nessuna fregatura: niente filigrana, niente account, niente abbonamento, niente caricamenti.

Tutto gira in locale su un motore nativo in Rust con un compositore su GPU. Installalo, trascina il girato, taglia. I modelli di IA per sottotitoli, voci e scontorno si scaricano una volta dalle Impostazioni e poi funzionano offline. Il tuo girato non lascia mai il tuo disco.

**Adatto a:** TikTok, Reels e Shorts, video YouTube, tutorial e registrazioni dello schermo, clip di podcast, meme.

**Anche per le macchine:** un’API JSON-RPC, così anche script e agenti IA possono montare video con Concat.

## In evidenza

- 🚫 **Niente filigrane. Niente account. Niente paywall.** Mai.
- 🔒 **100 % locale.** Niente viene caricato. Funziona offline.
- 💬 **Sottotitoli automatici.** Whisper in locale. Scegli la dimensione del modello e ottieni sottotitoli stilizzati sulla timeline.
- 🗣️ **Sintesi vocale + clonazione della voce.** Voci locali gratuite, o qualsiasi voce da pochi secondi di registrazione.
- 🧍 **Rimozione dello sfondo.** Persone, oggetti, o dipingi la maschera tu stesso.
- 🎞️ **Fotogrammi chiave.** Posizione, scala, rotazione, opacità, volume, parametri degli effetti. Editor di curve integrato.
- ✨ **100 effetti, filtri e transizioni.** Resi dalla GPU, dal vivo nell’anteprima.
- ✂️ **Taglia in fretta.** Dividi, rifila, ripple, unisci, fermo immagine, velocità. Timeline magnetica se la vuoi.
- 🎚️ **Multitraccia, multi-timeline.** Più montaggi in un progetto. Metodi di fusione, ritaglio, ribaltamenti.
- 📝 **Titoli.** Font, contorno, ombra, placca di sfondo. Preset da cui partire.
- 🎙️ **Pulizia della voce con un interruttore.** Riduci il rumore, migliora la voce, livella il volume. Più scoiattolo, robot, telefono e compagnia.
- 📤 **Esportazione.** H.264, HEVC, AV1. Fino a 4K 60, colore a 10 bit.
- 🦀 **Motore nativo in Rust.** Compositore su GPU, proxy, decodifica hardware. Il 4K scorre fluido.
- 🤖 **Scriptabile.** Un’API JSON-RPC con cui script e agenti IA possono montare video. Compilando dai sorgenti si aggiungono gRPC e uno strumento da riga di comando.
- 🖥️ **macOS, Windows, Linux, Android.** 26 lingue. Stessa app, stessi file di progetto.

## Download

Due strade:

1. **[Il sito web](https://concatenate.pages.dev/#download)** ti dà la build giusta per la tua macchina. Parti da qui.
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** ha ogni build per ogni piattaforma, con installer, pacchetti e checksum. Per quando vuoi scegliere.

Concat è in **beta**: funziona, e ha ancora qualche spigolo. [Segnalalo](https://github.com/jub0t/Concat/issues) quando ne trovi uno.

**Piattaforme**

- ✅ **Windows** · x86_64 (le build ARM sono in pausa, vedi [#326](https://github.com/jub0t/Concat/issues/326)). Un setup e un `.msi`. Se SmartScreen blocca una build non firmata: **Ulteriori informazioni** › **Esegui comunque**
- ✅ **macOS** · Intel e Apple Silicon. Se macOS si rifiuta di aprire una build non firmata: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 e ARM. `.deb`, `.rpm`, `.AppImage` e un pacchetto Arch. Anche un Flatpak su **[Flatpark](https://flatpark.org/apps/app.concat.editor/)**, un remote Flatpak della community che impacchetta il `.deb` x86_64 di ogni versione e si aggiorna con essa
- ✅ **Android** · telefoni e tablet
- ✅ **iOS / iPadOS** · iPhone e iPad, con sideload

✅ Supportato · 🚧 In lavorazione · 🧪 Da testare

I **requisiti di sistema** e le dimensioni dei modelli opzionali sono sul [sito web](https://concatenate.pages.dev/guides/system-requirements).

**[Code signing policy](../CODE_SIGNING.md)**

## Come contribuire

> [!IMPORTANT]
> Il modo migliore per contribuire è prendere una build dalla pagina [Releases](https://github.com/jub0t/Concat/releases) e usarla: trova dove si rompe, e di’ dove potrebbe essere migliore.
>
> Pronto a scrivere codice? [CONTRIBUTING.md](../CONTRIBUTING.md) copre la configurazione, la struttura dell’albero, i controlli da eseguire e come vengono licenziati i contributi. Guidi Concat da uno script, un servizio o un agente? [La documentazione per sviluppatori](https://concatenate.pages.dev/docs) copre l’API di Concat e i suoi trasporti: JSON-RPC e gRPC. [Questa Discussion](https://github.com/jub0t/Concat/discussions/3) è dove il progetto è stato annunciato.
> 
> I contributori possono ottenere un ruolo `@Contributor` sul server Discord: basta chiederlo.

## Storico delle stelle

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
</a>

## Contributori

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## DMCA

Se ritieni che qualcosa in questo repository, nelle sue release o in un pacchetto o modello che distribuisce violi il tuo diritto d’autore, scrivi a [jub0trd@gmail.com](mailto:jub0trd@gmail.com) indicando una descrizione dell’opera di cui sei titolare, un link al punto in cui il materiale contestato compare qui, i tuoi recapiti, una dichiarazione in cui affermi di ritenere in buona fede che l’uso non sia autorizzato da te, dal tuo rappresentante o dalla legge, e una dichiarazione, sotto pena di spergiuro, che la segnalazione è accurata e che sei il titolare o sei autorizzato ad agire per suo conto, con la tua firma autografa o elettronica. Le segnalazioni valide vengono gestite tempestivamente, e chiunque si veda rimuovere del materiale può inviare una contro-notifica allo stesso indirizzo.
